import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Mode = "text" | "supacast";
type Phase = "idle" | "listening" | "recording" | "transcribing" | "thinking" | "done" | "error";

const MAX_RECORD_MS = 60_000; // safety cap while holding Enter
const LONG_PRESS_MS = 300; // hold Enter this long before the ring records

export default function Dictate() {
  const [mode, setMode] = useState<Mode>("text");
  const [phase, setPhase] = useState<Phase>("idle");
  const [transcript, setTranscript] = useState("");
  const [answer, setAnswer] = useState("");
  const [error, setError] = useState("");
  const [level, setLevel] = useState(0);
  // Mirror of `phase` for event listeners that must not re-subscribe while a
  // physical key press is in progress (re-subscribing would reset refs).
  const phaseRef = useRef<Phase>("idle");
  phaseRef.current = phase;

  const streamRef = useRef<MediaStream | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const rafRef = useRef<number | undefined>(undefined);
  const hardTimer = useRef<number | undefined>(undefined);
  // Enter released while the mic was still starting up — stop as soon as it does.
  const releaseQueuedRef = useRef(false);
  // Tracks the physical Enter press shared by the in-window key events and
  // the global hotkey forwarded from the backend (both can fire at once).
  const pressActiveRef = useRef(false);
  const holdTimerRef = useRef<number | undefined>(undefined);

  const cleanupAudio = useCallback(() => {
    window.clearTimeout(hardTimer.current);
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    recorderRef.current?.stream.getTracks().forEach((t) => t.stop());
    recorderRef.current = null;
    audioCtxRef.current?.close().catch(() => {});
    audioCtxRef.current = null;
    streamRef.current?.getTracks().forEach((t) => t.stop());
    streamRef.current = null;
    setLevel(0);
  }, []);

  const finish = useCallback(
    async (cancelled: boolean) => {
      const recorder = recorderRef.current;
      if (cancelled || !recorder || recorder.state === "inactive") {
        // Discard the recording and exit dictation mode.
        cleanupAudio();
        void invoke("close_dictate");
        return;
      }
      setPhase("transcribing");
      const blob = await new Promise<Blob>((resolve) => {
        recorder.onstop = () => resolve(new Blob(chunksRef.current, { type: recorder.mimeType }));
        recorder.stop();
      });
      cleanupAudio();

      const b64 = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onloadend = () => {
          const result = String(reader.result);
          resolve(result.slice(result.indexOf(",") + 1)); // strip data: prefix
        };
        reader.onerror = reject;
        reader.readAsDataURL(blob);
      });

      try {
        const text = await invoke<string>("transcribe_audio", {
          audioBase64: b64,
          mime: blob.type || "audio/webm",
        }).then((t) => t.trim());

        if (!text) {
          setPhase("error");
          setError("Nothing heard");
          return;
        }
        setTranscript(text);

        if (mode === "text") {
          // Copies to clipboard, hides this window and pastes at the
          // cursor of whatever input was focused before dictating.
          await invoke("paste_to_focused_app", { text });
          setPhase("done");
        } else {
          setPhase("thinking");
          const reply = await invoke<string>("run_agent", { message: text });
          // Hand the Q&A to the launcher: it opens showing this exchange.
          await invoke("dictate_show_answer", { message: text, answer: reply });
          setPhase("done");
        }
      } catch (e) {
        setPhase("error");
        setError(String(e));
      }
    },
    [cleanupAudio, mode],
  );

  const start = useCallback(async () => {
    try {
      setError("");
      setTranscript("");
      setAnswer("");
      releaseQueuedRef.current = false;
      setPhase("listening");

      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;

      // Monitor the mic level so the ring pulses while recording.
      const ctx = new AudioContext();
      audioCtxRef.current = ctx;
      const source = ctx.createMediaStreamSource(stream);
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 512;
      source.connect(analyser);
      const data = new Uint8Array(analyser.frequencyBinCount);

      const monitor = () => {
        analyser.getByteTimeDomainData(data);
        let sum = 0;
        for (let i = 0; i < data.length; i++) {
          const v = (data[i] - 128) / 128;
          sum += v * v;
        }
        const rms = Math.sqrt(sum / data.length);
        setLevel(Math.min(1, rms * 4));
        rafRef.current = requestAnimationFrame(monitor);
      };
      rafRef.current = requestAnimationFrame(monitor);

      // Record everything; releasing Enter stops and transcribes.
      const recorder = new MediaRecorder(stream);
      recorderRef.current = recorder;
      chunksRef.current = [];
      recorder.ondataavailable = (e) => chunksRef.current.push(e.data);
      recorder.start();

      hardTimer.current = window.setTimeout(() => finish(false), MAX_RECORD_MS);
      setPhase("recording");

      // Enter was released before the mic finished starting up.
      if (releaseQueuedRef.current) {
        releaseQueuedRef.current = false;
        finish(false);
      }
    } catch (e) {
      setPhase("error");
      setError(String(e));
    }
  }, [finish]);

  // Receive the requested mode from the launcher, then show it and wait.
  // Recording only starts when the user holds Enter (push-to-talk).
  useEffect(() => {
    const unlisten = listen<string>("dictate-mode", (e) => {
      const m: Mode = e.payload === "text" ? "text" : "supacast";
      setMode(m);
      setPhase("idle");
      setError("");
      setTranscript("");
      setAnswer("");
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Opening the launcher exits dictation mode.
  useEffect(() => {
    const unlisten = listen("launcher-shown", () => {
      cleanupAudio();
      void invoke("close_dictate");
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [cleanupAudio]);

  useEffect(() => () => cleanupAudio(), [cleanupAudio]);

  // Keyboard: Esc/Enter work at window level regardless of which element
  // (if any) has DOM focus. Enter is also forwarded from a global hotkey
  // (see set_dictate_capture in the backend) so holding Enter records even
  // when the ring window itself has no keyboard focus.
  //
  // This effect must NOT depend on `phase`: re-subscribing mid-press would
  // reset the press-tracking refs and drop the release event (the recording
  // would then never stop). Handlers read the live phase from `phaseRef`.
  useEffect(() => {
    // Hold Enter for LONG_PRESS_MS before recording starts, so quick Enter
    // taps in other apps are unaffected; release stops and transcribes.
    const beginHold = () => {
      if (pressActiveRef.current) return; // same physical press
      const phase = phaseRef.current;
      if (phase !== "idle" && phase !== "done" && phase !== "error") return;
      pressActiveRef.current = true;
      holdTimerRef.current = window.setTimeout(() => {
        holdTimerRef.current = undefined;
        if (pressActiveRef.current) start();
      }, LONG_PRESS_MS);
    };
    const endHold = () => {
      if (!pressActiveRef.current) return;
      pressActiveRef.current = false;
      if (holdTimerRef.current !== undefined) {
        // Released before the long-press threshold — ignore entirely.
        window.clearTimeout(holdTimerRef.current);
        holdTimerRef.current = undefined;
        return;
      }
      const phase = phaseRef.current;
      if (phase === "recording") {
        finish(false); // released → transcribe and do the needful
      } else if (phase === "listening") {
        releaseQueuedRef.current = true; // mic still starting; stop when ready
      }
    };

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Stop recording (if any) and exit dictation mode.
        e.preventDefault();
        finish(true);
      } else if (e.key === "Enter") {
        e.preventDefault();
        beginHold();
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key === "Enter") endHold();
    };

    const unlisteners = [
      listen<string>("dictate-key", (e) => {
        if (e.payload === "down") beginHold();
        else endHold();
      }),
    ];
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      unlisteners.forEach((p) => p.then((fn) => fn()));
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [finish, start]);

  /// Cancel any in-flight recording (or just close the ring) via the ✕.
  const closeRing = () => {
    const phase = phaseRef.current;
    if (phase === "recording" || phase === "listening") {
      finish(true); // discards the recording and closes
    } else {
      void invoke("close_dictate");
    }
  };

  // Drag the ring around the screen. A plain click on a finished/errored
  // ring exits instead of dragging.
  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    if (phase === "done" || phase === "error") {
      void invoke("close_dictate");
      return;
    }
    getCurrentWindow().startDragging();
  };

  const orbInner = () => {
    switch (phase) {
      case "idle":
        return (
          <span className="orb-hint">
            Hold
            <kbd>Enter</kbd>
          </span>
        );
      case "listening":
      case "recording":
        return <span className="orb-icon">🎙</span>;
      case "transcribing":
        return (
          <span className="thinking-dots">
            <span />
            <span />
            <span />
          </span>
        );
      case "thinking":
        return <span className="orb-icon">🚀</span>;
      case "done":
        return <span className="orb-icon ok">✓</span>;
      case "error":
        // Short messages (like "Nothing heard.") fit inside the ring;
        // long API errors keep the "!" mark and show below.
        return error.length <= 32 ? (
          <span className="orb-msg err">{error}</span>
        ) : (
          <span className="orb-icon err">!</span>
        );
    }
  };

  return (
    <div className="dictate" onMouseDown={onMouseDown}>
      <div
        className={`orb ${phase} ${mode === "supacast" ? "supacast" : "text"}`}
        style={phase === "recording" ? { transform: `scale(${1 + level * 0.35})` } : undefined}
      >
        <div className="orb-core">{orbInner()}</div>
        <button
          className="orb-close"
          title="Close dictate"
          onMouseDown={(e) => e.stopPropagation()}
          onClick={closeRing}
        >
          ✕
        </button>
      </div>
      {transcript && <p className="dictate-transcript">“{transcript}”</p>}
      {answer && <pre className="agent-answer">{answer}</pre>}
      {error && error.length > 32 && <p className="status err">{error}</p>}
    </div>
  );
}
