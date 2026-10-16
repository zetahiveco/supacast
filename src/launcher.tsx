import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface SearchResult {
  title: string;
  subtitle: string;
  path: string;
  kind: "app" | "file" | "folder" | "todo";
}

interface Todo {
  id: string;
  text: string;
  due: string | null;
  done: boolean;
}

interface Note {
  id: string;
  text: string;
  created: string | null;
}

interface ClipEntry {
  text: string;
  at: string;
}

interface Dictation {
  id: string;
  text: string;
  answer?: string | null;
  mime?: string;
  created?: string | null;
  duration_ms?: number | null;
}

interface ChatTurn {
  role: "user" | "assistant";
  content: string;
  thinking?: string;
  tools?: string[];
  streaming?: boolean;
  error?: boolean;
  /** True until the first stream event arrives. */
  waiting?: boolean;
  /** Wall-clock ms since the turn started (for the progress readout). */
  startedAt?: number;
  elapsed?: number;
}

type View =
  | { kind: "search" }
  | { kind: "todos" }
  | { kind: "notes"; query?: string }
  | { kind: "clipboard" }
  | { kind: "dictations" }
  | { kind: "chat" };

/** Route the raw query to a command view. */
function route(q: string): View | null {
  const lower = q.trim().toLowerCase();

  // "todo"/"todos" (optionally "todo tomorrow", etc.) shows the full
  // date-wise list — overdue, today, tomorrow, upcoming, someday.
  if (/^todos?(\s|$)/i.test(lower)) {
    return { kind: "todos" };
  }
  // "notes" lists all notes; "notes wifi" filters them.
  // "note …" (singular + text) stays in search so the add-note suggestion shows.
  if (/^notes?$/i.test(lower)) {
    return { kind: "notes" };
  }
  const noteFilter = q.trim().match(/^notes\s+(.+)/i);
  if (noteFilter) {
    return { kind: "notes", query: noteFilter[1] };
  }
  if (/^clipboard(\s+history)?/i.test(lower)) {
    return { kind: "clipboard" };
  }
  // "dictations" (or "dictation history") lists saved dictation recordings.
  if (/^dictations?(\s+history)?$/i.test(lower)) {
    return { kind: "dictations" };
  }
  if (/^dictate\s*\(?(text|txt)\)?/i.test(lower)) {
    return { kind: "chat" }; // handled by Enter -> dictate
  }
  if (/^dictate\s*\(?(supacast|ai)?\)?$/i.test(lower)) {
    return { kind: "chat" };
  }
  return null;
}

export default function Launcher() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [active, setActive] = useState(0);
  const [searching, setSearching] = useState(false);
  const [view, setView] = useState<View>({ kind: "search" });
  const [todos, setTodos] = useState<Todo[]>([]);
  const [notes, setNotes] = useState<Note[]>([]);
  const [clips, setClips] = useState<ClipEntry[]>([]);
  // Dictation recordings view: which row is expanded + its audio data URL.
  const [dictations, setDictations] = useState<Dictation[]>([]);
  const [selDict, setSelDict] = useState<string | null>(null);
  const [dictAudio, setDictAudio] = useState<{ id: string; url: string } | null>(null);
  // Expanded note in the notes view (long notes are clipped to one line).
  const [selNote, setSelNote] = useState<string | null>(null);
  // Chat state (Ask Supacast)
  const [chat, setChat] = useState<ChatTurn[]>([]);
  const [chatBusy, setChatBusy] = useState(false);
  // Ticks while the AI is streaming so elapsed-time progress updates.
  const [nowTick, setNowTick] = useState(Date.now());
  // True while the chat thread originated from a dictate hand-off;
  // empty-Enter then re-enters dictation mode.
  const [dictateThread, setDictateThread] = useState(false);
  useEffect(() => {
    if (!chatBusy) return;
    const id = window.setInterval(() => setNowTick(Date.now()), 250);
    return () => window.clearInterval(id);
  }, [chatBusy]);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<number | undefined>(undefined);
  const searchIdRef = useRef(0);
  const chatScrollRef = useRef<HTMLDivElement>(null);
  // Scrollable results list (todos/notes/clipboard/dictations/search — only
  // one renders at a time, so one ref serves all views).
  const listRef = useRef<HTMLUListElement>(null);
  const chatRef = useRef<ChatTurn[]>([]);
  chatRef.current = chat; // updated on every render, safe to read in callbacks
  // Rows must only be hover-selected when the mouse actually moved. Without
  // this, rows rendering under a stationary cursor steal the selection and
  // Enter opens/hides something the user never pointed at.
  const mouseMovedRef = useRef(false);

  useEffect(() => {
    const onMove = () => {
      mouseMovedRef.current = true;
    };
    window.addEventListener("mousemove", onMove);
    return () => window.removeEventListener("mousemove", onMove);
  }, []);

  /** Hover-select only after real mouse movement. */
  const hover = useCallback((i: number) => {
    if (mouseMovedRef.current) setActive(i);
  }, []);

  const reset = useCallback(() => {
    searchIdRef.current++; // cancel any in-flight search
    setQuery("");
    setResults([]);
    setActive(0);
    setView({ kind: "search" });
    setTodos([]);
    setNotes([]);
    setClips([]);
    setDictations([]);
    setSelDict(null);
    setDictAudio(null);
    // A fresh launcher session: drop the previous chat thread (it stays
    // available under "dictations"). Without this, leftover chat history
    // used to block todo/notes/app/file search on every later keystroke.
    setChat([]);
    setDictateThread(false);
    requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  useEffect(() => {
    const unlisten = listen("launcher-shown", () => reset());
    reset();
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [reset]);

  // Tray "Dictate" item jumps straight to dictate mode.
  useEffect(() => {
    const unlisten = listen<string>("open-dictate", (e) => {
      invoke("open_dictate", { mode: e.payload });
      invoke("hide_launcher");
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // A finished Dictate (Supacast) hands its Q&A over: the launcher is
  // already visible (shown by dictate_show_answer) — display the exchange.
  useEffect(() => {
    const unlisten = listen<{ message: string; answer: string }>("dictate-answer", (e) => {
      setChat((prev) => [
        ...prev,
        { role: "user", content: e.payload.message },
        { role: "assistant", content: e.payload.answer },
      ]);
      setDictateThread(true);
      setView({ kind: "chat" });
      setQuery("");
      // Focus can silently fail if the webview isn't key yet — retry once.
      requestAnimationFrame(() => inputRef.current?.focus());
      window.setTimeout(() => inputRef.current?.focus(), 150);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Debounced app/file search (only in search view).
  useEffect(() => {
    // In the chat view typing edits the next message, so no search. Guarded
    // on the *view*, not chat history — a leftover thread must never stop
    // todo/notes/app/file search from working.
    if (view.kind === "chat") return;
    const q = query.trim();
    if (!q) return; // empty input: keep the current view (e.g. after add todo/note)
    const routed = route(q);
    setView(routed ?? { kind: "search" });
    mouseMovedRef.current = false; // new list: don't inherit stale hover

    if (routed) {
      // Invalidate any in-flight search so it can't land later.
      searchIdRef.current++;
      setResults([]);
      setSearching(false);
      return;
    }

    // Typing just "ai" and pausing enters AI chat mode with a fresh input.
    // A short delay keeps fast typing (e.g. "airdrop") in normal search:
    // any further keystroke before the timer fires cancels the switch.
    if (/^ai$/i.test(q)) {
      const t = window.setTimeout(() => {
        setView({ kind: "chat" });
        setQuery("");
      }, 350);
      return () => window.clearTimeout(t);
    }

    setSearching(true);
    window.clearTimeout(debounceRef.current);
    const searchId = ++searchIdRef.current;
    debounceRef.current = window.setTimeout(async () => {
      try {
        const [apps, files] = await Promise.all([
          invoke<SearchResult[]>("search_apps", { query: q }),
          invoke<SearchResult[]>("search_files", { query: q }),
        ]);
        // Ignore stale responses: a slower older search must not
        // overwrite the results of a newer one.
        if (searchId !== searchIdRef.current) return;
        setResults([...apps, ...files]);
        setActive(0);
      } catch (e) {
        console.error("search failed", e);
      } finally {
        if (searchId === searchIdRef.current) setSearching(false);
      }
    }, 150);
    return () => window.clearTimeout(debounceRef.current);
  }, [query, view.kind]);

  // Load data when entering the todos/notes/clipboard views.
  useEffect(() => {
    if (view.kind === "todos") {
      invoke<Todo[]>("list_todos", { scope: "all" }).then(setTodos).catch(console.error);
      setActive(0);
    } else if (view.kind === "notes") {
      invoke<Note[]>("list_notes", { query: view.query ?? "" })
        .then(setNotes)
        .catch(console.error);
      setActive(0);
      setSelNote(null);
    } else if (view.kind === "clipboard") {
      invoke<ClipEntry[]>("get_clipboard_history").then(setClips).catch(console.error);
      setActive(0);
    } else if (view.kind === "search") {
      // Empty launcher: pending todos headline the list, so keep them
      // loaded. "clear …" needs the clip count for its confirm subtitle.
      if (!query.trim()) {
        invoke<Todo[]>("list_todos", { scope: "all" }).then(setTodos).catch(console.error);
      } else if (/^clear/i.test(query.trim())) {
        invoke<ClipEntry[]>("get_clipboard_history").then(setClips).catch(console.error);
      }
    } else if (view.kind === "dictations") {
      invoke<Dictation[]>("list_dictations", { query: null })
        .then(setDictations)
        .catch(console.error);
      setActive(0);
      setSelDict(null);
      setDictAudio(null);
    }
  }, [view]);

  // Keep the chat scrolled to the newest message.
  useEffect(() => {
    chatScrollRef.current?.scrollTo({ top: chatScrollRef.current.scrollHeight });
  }, [chat]);

  // Keep the keyboard-selected row visible when navigating with ↑/↓ (or on
  // hover). "nearest" avoids jumping when the row is already on screen.
  useEffect(() => {
    listRef.current
      ?.querySelector(".result.active")
      ?.scrollIntoView({ block: "nearest" });
  }, [active, view]);

  const open = useCallback(async (item: SearchResult) => {
    try {
      await invoke("open_path", { path: item.path });
      await invoke("hide_launcher");
    } catch (e) {
      console.error("failed to open", e);
    }
  }, []);

  const openDictate = useCallback((mode: "text" | "supacast") => {
    invoke("open_dictate", { mode });
    invoke("hide_launcher");
  }, []);

  const copyClip = useCallback(async (text: string) => {
    await invoke("copy_to_clipboard", { text });
    invoke("hide_launcher");
  }, []);

  /** Clear all button / command: wipe the clipboard history in place. */
  const clearAllClips = useCallback(() => {
    invoke("clear_clipboard_history").then(() => setClips([])).catch(console.error);
  }, []);

  /** Select (or deselect) a dictation: expands the full transcript and
   * loads its recording for the inline player. */
  const toggleDictation = useCallback(
    async (d: Dictation) => {
      if (selDict === d.id) {
        setSelDict(null);
        setDictAudio(null);
        return;
      }
      setSelDict(d.id);
      setDictAudio(null);
      try {
        const url = await invoke<string>("get_dictation_audio", { id: d.id });
        setDictAudio({ id: d.id, url });
      } catch (e) {
        console.error("failed to load recording", e);
      }
    },
    [selDict],
  );

  const deleteDictation = useCallback(
    async (id: string) => {
      await invoke("delete_dictation", { id });
      if (selDict === id) {
        setSelDict(null);
        setDictAudio(null);
      }
      invoke<Dictation[]>("list_dictations", { query: null })
        .then(setDictations)
        .catch(console.error);
    },
    [selDict],
  );

  const toggleTodo = useCallback(async (todo: Todo) => {
    // Check ⇄ uncheck (keeps the todo; unchecking also re-arms reminders).
    await invoke("set_todo_done", { id: todo.id, done: !todo.done });
    invoke<Todo[]>("list_todos", { scope: "all" }).then(setTodos).catch(console.error);
  }, []);

  const deleteTodo = useCallback(async (id: string) => {
    await invoke("delete_todo", { id });
    invoke<Todo[]>("list_todos", { scope: "all" }).then(setTodos).catch(console.error);
  }, []);

  /** Select (or deselect) a note: expands it below the row to show the
   * full text — long notes are clipped to one line in the list. */
  const toggleNote = useCallback((n: Note) => {
    setSelNote((sel) => (sel === n.id ? null : n.id));
  }, []);

  const deleteNote = useCallback(
    async (id: string) => {
      await invoke("delete_note", { id });
      setSelNote((sel) => (sel === id ? null : sel));
      const query = view.kind === "notes" ? (view.query ?? "") : "";
      invoke<Note[]>("list_notes", { query }).then(setNotes).catch(console.error);
    },
    [view],
  );

  /** Enter on a search that looks like a question starts a chat turn. */
  const sendChat = useCallback(
    async (text: string) => {
      if (!text.trim() || chatBusy) return;
      setView({ kind: "chat" });
      setQuery("");

      const userTurn: ChatTurn = { role: "user", content: text.trim() };
      const assistantTurn: ChatTurn = {
        role: "assistant",
        content: "",
        thinking: "",
        tools: [],
        streaming: true,
        waiting: true,
        startedAt: Date.now(),
      };
      const history: ChatTurn[] = [...chatRef.current, userTurn, assistantTurn];
      setChat(history);
      setChatBusy(true);

      try {
        const channel = new Channel<{
          type: string;
          text?: string;
          name?: string;
          message?: string;
        }>();
        channel.onmessage = (event) => {
          setChat((prev) => {
            const next = [...prev];
            const last = { ...next[next.length - 1], waiting: false };
            switch (event.type) {
              case "start":
                break;
              case "thinking":
                last.thinking = (last.thinking || "") + (event.text || "");
                break;
              case "delta":
                last.content += event.text || "";
                break;
              case "tool":
                last.tools = [...(last.tools || []), event.name || "tool"];
                break;
              case "done":
                if (event.text) last.content = event.text;
                last.streaming = false;
                last.elapsed = Date.now() - (last.startedAt || Date.now());
                break;
              case "error":
                last.content = event.message || "Something went wrong.";
                last.error = true;
                last.streaming = false;
                break;
            }
            next[next.length - 1] = last;
            return next;
          });
        };

        await invoke("chat_stream", {
          history: history.slice(0, -1).map(({ role, content }) => ({ role, content })),
          message: text.trim(),
          channel,
        });
      } catch (e) {
        setChat((prev) => {
          const next = [...prev];
          next[next.length - 1] = {
            ...next[next.length - 1],
            content: String(e),
            error: true,
            streaming: false,
          };
          return next;
        });
      } finally {
        setChatBusy(false);
        setChat((prev) => {
          if (!prev[prev.length - 1]?.streaming) return prev;
          const next = [...prev];
          next[next.length - 1] = { ...next[next.length - 1], streaming: false };
          return next;
        });
      }
    },
    [chatBusy],
  );

  const clearChat = useCallback(() => {
    setChat([]);
    setDictateThread(false);
    setView({ kind: "search" });
    inputRef.current?.focus();
  }, []);

  const handleSpecial = useCallback(
    (path: string) => {
      if (path.startsWith("__add_todo__:")) {
        const text = path.slice("__add_todo__:".length);
        invoke("add_todo", { text, due: null })
          .then(() => {
            // Stay open: show the todos list as feedback.
            setQuery("");
            setView({ kind: "todos" });
            requestAnimationFrame(() => inputRef.current?.focus());
          })
          .catch(console.error);
        return true;
      }
      if (path.startsWith("__add_note__:")) {
        const text = path.slice("__add_note__:".length);
        invoke("add_note", { text })
          .then(() => {
            // Stay open: show the notes list as feedback.
            setQuery("");
            setView({ kind: "notes" });
            requestAnimationFrame(() => inputRef.current?.focus());
          })
          .catch(console.error);
        return true;
      }
      if (path.startsWith("__remind__:")) {
        const [text, iso] = path.slice("__remind__:".length).split("|");
        invoke("add_todo", { text, due: iso })
          .then(() => {
            // Stay open: reminders are todos — show the list.
            setQuery("");
            setView({ kind: "todos" });
            requestAnimationFrame(() => inputRef.current?.focus());
          })
          .catch(console.error);
        return true;
      }
      // Empty-state command list: jump straight to a section.
      if (path.startsWith("__view_")) {
        const kind = path.slice("__view_".length) as
          | "todos"
          | "notes"
          | "clipboard"
          | "dictations";
        setQuery("");
        setView({ kind });
        requestAnimationFrame(() => inputRef.current?.focus());
        return true;
      }
      // "ai" from the empty-state command list: enter chat with no thread.
      if (path === "__chat__") {
        setQuery("");
        setChat([]);
        setDictateThread(false);
        setView({ kind: "chat" });
        requestAnimationFrame(() => inputRef.current?.focus());
        return true;
      }
      // "clear clipboard": wipe the history, then show the emptied view
      // as inline feedback instead of a notification banner.
      if (path === "__clear_clipboard__") {
        invoke("clear_clipboard_history")
          .then(() => {
            setClips([]);
            setQuery("");
            setView({ kind: "clipboard" });
            requestAnimationFrame(() => inputRef.current?.focus());
          })
          .catch(console.error);
        return true;
      }
      // Syntax examples: pre-fill the query so the user finishes typing.
      if (path === "__cmd_add_todo__") {
        setQuery("add todo ");
        inputRef.current?.focus();
        return true;
      }
      if (path === "__cmd_add_note__") {
        setQuery("note: ");
        inputRef.current?.focus();
        return true;
      }
      if (path === "__cmd_remind__") {
        setQuery("remind me to ");
        inputRef.current?.focus();
        return true;
      }
      return false;
    },
    [],
  );

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      invoke("hide_launcher");
      return;
    }

    // In chat view, Enter sends the message (input is at the top like the widget).
    if (view.kind === "chat") {
      if (e.key === "Enter") {
        e.preventDefault();
        if (!query.trim() && dictateThread) {
          // Empty Enter in a dictated thread → back into dictation mode.
          openDictate("supacast");
        } else if (!chat.length && isDictateQuery) {
          // "dictate (text)"/"dictate (supacast)" query — Enter starts that
          // mode, matching the hint shown for this state.
          openDictate(/text/i.test(query) ? "text" : "supacast");
        } else {
          sendChat(query);
        }
      } else if (e.key === "Backspace" && !query) {
        // Empty input + Backspace leaves the chat and returns to search.
        // The thread is kept — typing "ai" re-enters it.
        e.preventDefault();
        setView({ kind: "search" });
      }
      return;
    }

    if (e.key === "ArrowDown") {
      e.preventDefault();
      mouseMovedRef.current = false; // keyboard selection wins over hover
      const len =
        view.kind === "search"
          ? searchRows.length
          : view.kind === "todos"
            ? flatTodos.length
            : view.kind === "notes"
              ? notes.length
              : view.kind === "dictations"
                ? dictations.length
                : clips.length;
      setActive((a) => Math.min(a + 1, len - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      mouseMovedRef.current = false;
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (view.kind === "search") {
        // "ai" + Enter jumps straight into AI chat mode (no need to wait
        // for the pause-based switch).
        if (/^ai$/i.test(query.trim())) {
          setView({ kind: "chat" });
          setQuery("");
          return;
        }
        const item = searchRows[active];
        if (!item) return;
        if (handleSpecial(item.path)) return;
        if (item.path === "__ask__") {
          sendChat(query.trim());
        } else if (item.path === "__dictate_text__") {
          openDictate("text");
        } else if (item.path === "__dictate_supacast__") {
          openDictate("supacast");
        } else {
          open(item);
        }
      } else if (view.kind === "todos" && flatTodos[active]) {
        toggleTodo(flatTodos[active]);
      } else if (view.kind === "notes" && notes[active]) {
        // Enter expands the note (long ones are clipped in the list);
        // copying is available from the expanded detail.
        toggleNote(notes[active]);
      } else if (view.kind === "clipboard" && clips[active]) {
        copyClip(clips[active].text);
      } else if (view.kind === "dictations" && dictations[active]) {
        // Enter expands the selected dictation and shows the player.
        toggleDictation(dictations[active]);
      }
    }
  };

  const fmtDue = (iso: string | null) => {
    if (!iso) return "";
    try {
      return new Date(iso).toLocaleString([], {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      });
    } catch {
      return iso;
    }
  };

  /** Recording length as m:ss. */
  const fmtDur = (ms: number | null | undefined) => {
    if (!ms || ms <= 0) return "";
    const total = Math.round(ms / 1000);
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
  };

  // --- Date-wise todo grouping (Overdue / Today / Tomorrow / ...) ---
  const startOfDay = (d: Date) =>
    new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const today0 = startOfDay(new Date());
  const tomorrow0 = new Date(today0);
  tomorrow0.setDate(tomorrow0.getDate() + 1);

  const todoGroups: { label: string; items: Todo[] }[] = [
    { label: "Overdue", items: [] },
    { label: "Today", items: [] },
    { label: "Tomorrow", items: [] },
    { label: "Upcoming", items: [] },
    { label: "Someday", items: [] },
    { label: "Completed", items: [] },
  ];
  for (const t of todos) {
    if (t.done) {
      todoGroups[5].items.push(t);
      continue;
    }
    if (!t.due) {
      todoGroups[4].items.push(t);
      continue;
    }
    const due = startOfDay(new Date(t.due));
    if (due < today0) todoGroups[0].items.push(t);
    else if (due.getTime() === today0.getTime()) todoGroups[1].items.push(t);
    else if (due.getTime() === tomorrow0.getTime()) todoGroups[2].items.push(t);
    else todoGroups[3].items.push(t);
  }
  for (const g of todoGroups) {
    g.items.sort((a, b) => (a.due ?? "").localeCompare(b.due ?? ""));
  }
  const visibleGroups = todoGroups.filter((g) => g.items.length > 0);
  const flatTodos = visibleGroups.flatMap((g) => g.items);

  // Suggestions shown while typing in search view.
  const suggestions: SearchResult[] = [];
  if (view.kind === "search" && query.trim()) {
    // Local todo commands (work without the AI agent).
    const addMatch = query.trim().match(/^add\s+todo\s+(.+)/i);
    const addNoteMatch = query.trim()
      .match(/^(?:add\s+note|note)\s*:?\s+(?:that\s+)?(.+)/i);
    const remindMatch = query
      .trim()
      .match(/^(?:remind me(?:\s+to)?\s+)(.+?)\s+(?:at|by)\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/i);
    if (addMatch) {
      suggestions.push({
        title: `Add todo: “${addMatch[1]}”`,
        subtitle: "Press Enter to save — no AI needed",
        path: `__add_todo__:${addMatch[1]}`,
        kind: "app",
      });
    }
    if (remindMatch) {
      let h = parseInt(remindMatch[2], 10);
      const m = remindMatch[3] ? parseInt(remindMatch[3], 10) : 0;
      const ap = remindMatch[4]?.toLowerCase();
      if (ap === "pm" && h < 12) h += 12;
      if (ap === "am" && h === 12) h = 0;
      const due = new Date();
      due.setHours(h, m, 0, 0);
      if (due < new Date()) due.setDate(due.getDate() + 1); // "at 5pm" tomorrow if past
      suggestions.push({
        title: `Reminder: “${remindMatch[1]}” → ${due.toLocaleString(
          [],
          { hour: "numeric", minute: "2-digit" },
        )}`,
        subtitle: "Press Enter to save — notifies you when due",
        path: `__remind__:${remindMatch[1]}|${due.toISOString()}`,
        kind: "app",
      });
    }
    if (addNoteMatch) {
      suggestions.push({
        title: `Add note: “${addNoteMatch[1]}”`,
        subtitle: "Press Enter to save — no AI needed",
        path: `__add_note__:${addNoteMatch[1]}`,
        kind: "app",
      });
    }
    suggestions.push({
      title: `Ask Supacast: “${query.trim()}”`,
      subtitle: "AI chat — todos, reminders, calendar",
      path: "__ask__",
      kind: "app",
    });
    suggestions.push({
      title: `Dictate (Text)`,
      subtitle: "Record & transcribe → clipboard",
      path: "__dictate_text__",
      kind: "app",
    });
    suggestions.push({
      title: `Dictate (Supacast)`,
      subtitle: "Record & ask the Supacast AI",
      path: "__dictate_supacast__",
      kind: "app",
    });
    // "clear clipboard" — typed command to wipe the clip history.
    if (/^clear/i.test(query.trim())) {
      suggestions.push({
        title: "Clear clipboard history",
        subtitle: `Deletes all ${clips.length} clips — press Enter to confirm`,
        path: "__clear_clipboard__",
        kind: "app",
      });
    }
  }
  const allResults = [...suggestions, ...results];

  // Empty search bar: pending todos first (dated soonest at the top, then
  // undated), followed by every built-in command — the launcher's browsable
  // reference, mirroring what you can type.
  const emptyQuery = view.kind === "search" && !query.trim();
  const emptyStateRows: SearchResult[] = [];
  if (emptyQuery) {
    const pending = todos.filter((t) => !t.done);
    // Dated todos first — the sort is stable so ties keep their saved order.
    pending.sort((a, b) => Number(!a.due) - Number(!b.due));
    const shown = pending.slice(0, 5).map((t) => ({
      title: t.text,
      subtitle: t.due ? fmtDue(t.due) : "No due date",
      path: "__view_todos__",
      kind: "todo" as const,
    }));
    emptyStateRows.push(...shown);
    const hidden = pending.length - shown.length;
    if (hidden > 0) {
      emptyStateRows.push({
        title: `+${hidden} more todo${hidden === 1 ? "" : "s"}`,
        subtitle: "Open Todos to see everything",
        path: "__view_todos__",
        kind: "todo",
      });
    }
    emptyStateRows.push(
      { title: "todo", subtitle: "Show your todo list — toggle & delete inline", path: "__view_todos__", kind: "app" },
      { title: "notes", subtitle: "Browse notes — “notes wifi” filters them", path: "__view_notes__", kind: "app" },
      { title: "clipboard history", subtitle: "Recent clips — Enter copies again", path: "__view_clipboard__", kind: "app" },
      { title: "clear clipboard", subtitle: "Delete the entire clipboard history", path: "__clear_clipboard__", kind: "app" },
      { title: "dictations", subtitle: "Transcript history with inline playback", path: "__view_dictations__", kind: "app" },
      { title: "add todo <text>", subtitle: "Quick-add a todo — no AI needed", path: "__cmd_add_todo__", kind: "app" },
      { title: "note: <text>", subtitle: "Quick-save a note — no AI needed", path: "__cmd_add_note__", kind: "app" },
      { title: "remind me to <text> at 5pm", subtitle: "Time-based reminder — notifies when due", path: "__cmd_remind__", kind: "app" },
      { title: "ai", subtitle: "Ask Supacast — todos, reminders, calendar", path: "__chat__", kind: "app" },
      { title: "dictate (text)", subtitle: "Record & transcribe → clipboard", path: "__dictate_text__", kind: "app" },
      { title: "dictate (supacast)", subtitle: "Record & ask the Supacast AI", path: "__dictate_supacast__", kind: "app" },
    );
  }
  const searchRows = emptyQuery ? emptyStateRows : allResults;

  const isDictateQuery = /^dictate\s*\(/i.test(query.trim());

  return (
    <div className="launcher">
      <div className="search-row">
        <span className="rocket">🚀</span>
        <input
          ref={inputRef}
          className="search-input"
          placeholder={
            view.kind === "chat"
              ? chat.length > 0
                ? "Reply to Supacast…"
                : "Ask Supacast…"
              : "Search, or type a command…"
          }
          value={query}
          autoFocus
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
        />
        {searching && <span className="spinner" />}
        {view.kind === "chat" && chat.length > 0 && (
          <button className="ghost-btn" onClick={clearChat} title="Clear chat">
            Clear
          </button>
        )}
      </div>

      {/* ---- Chat view (Ask Supacast) ---- */}
      {view.kind === "chat" && chat.length > 0 && (
        <div className="chat" ref={chatScrollRef}>
          {chat.map((turn, i) => (
            <div key={i} className={`chat-turn ${turn.role}`}>
              {turn.role === "user" ? (
                <div className="chat-bubble user">{turn.content}</div>
              ) : (
                <div className={`chat-bubble ai ${turn.error ? "err" : ""}`}>
                  {turn.tools && turn.tools.length > 0 && (
                    <div className="tool-uses">
                      {turn.tools.map((t, j) => (
                        <span key={j} className="tool-chip">
                          🔧 {t}
                        </span>
                      ))}
                    </div>
                  )}

                  {/* Progress while waiting for the first byte / thinking. */}
                  {turn.streaming && !turn.content && (
                    <div className="chat-progress">
                      {turn.waiting ? (
                        <>
                          <span className="spinner" /> Connecting to model…
                        </>
                      ) : (
                        <>
                          <span className="thinking-dots">
                            <span /><span /><span />
                          </span>
                          Thinking
                        </>
                      )}
                      <span className="elapsed">
                        {Math.max(0, Math.round((nowTick - (turn.startedAt || nowTick)) / 1000))}s
                      </span>
                    </div>
                  )}

                  {turn.thinking && turn.thinking.trim() && (
                    <details className="thinking" open={turn.streaming && !turn.content}>
                      <summary>Thinking</summary>
                      <pre>{turn.thinking}</pre>
                    </details>
                  )}
                  {turn.content}
                  {turn.streaming && turn.content && <span className="caret" />}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {view.kind === "chat" && chat.length === 0 && (
        <div className="agent-view">
          <p className="agent-hint">
            {isDictateQuery
              ? `Press Enter to start ${
                  /text/i.test(query) ? "Dictate (Text)" : "Dictate (Supacast)"
                }…`
              : "Ask Supacast — type a question and press Enter • Backspace to go back"}
          </p>
        </div>
      )}

      {/* ---- Todos view (date-wise, checkable) ---- */}
      {view.kind === "todos" && (
        <ul className="results" ref={listRef}>
          {visibleGroups.map((g) => {
            let idx = flatTodos.findIndex((t) => t.id === g.items[0].id);
            return (
              <li key={g.label} className="todo-group">
                <span className="todo-group-label">{g.label}</span>
                <ul className="todo-items">
                  {g.items.map((t) => {
                    const i = idx++;
                    return (
                      <li
                        key={t.id}
                        className={`result ${i === active ? "active" : ""}`}
                        onMouseEnter={() => hover(i)}
                        onClick={() => toggleTodo(t)}
                      >
                        <span
                          className={`todo-check ${t.done ? "on" : ""}`}
                          role="checkbox"
                          aria-checked={t.done}
                        >
                          {t.done ? "✓" : ""}
                        </span>
                        <span className={`result-title ${t.done ? "todo-done" : ""}`}>
                          {t.text}
                        </span>
                        <span className="result-sub">{t.due ? fmtDue(t.due) : ""}</span>
                        <button
                          className="row-trash"
                          title="Delete todo"
                          onClick={(e) => {
                            e.stopPropagation();
                            deleteTodo(t.id);
                          }}
                        >
                          🗑
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </li>
            );
          })}
          {flatTodos.length === 0 && (
            <li className="empty">No todos yet — try “add todo …” or ask the AI</li>
          )}
        </ul>
      )}

      {/* ---- Notes view ---- */}
      {view.kind === "notes" && (
        <ul className="results" ref={listRef}>
          {notes.map((n, i) => {
            const selected = selNote === n.id;
            return (
              <li
                key={n.id}
                className={`result note ${i === active ? "active" : ""} ${selected ? "selected" : ""}`}
                onMouseEnter={() => hover(i)}
                onClick={() => toggleNote(n)}
              >
                {/* One line per note; long text is clipped with an ellipsis. */}
                <div className="note-row">
                  <span className="badge note">Note</span>
                  <span className="result-title note-text">{n.text}</span>
                  <span className="result-sub">{fmtDue(n.created)}</span>
                  <button
                    className="row-trash"
                    title="Delete note"
                    onClick={(e) => {
                      e.stopPropagation();
                      deleteNote(n.id);
                    }}
                  >
                    🗑
                  </button>
                </div>
                {/* Selected: full note text + copy action. */}
                {selected && (
                  <div className="note-detail" onClick={(e) => e.stopPropagation()}>
                    <p className="note-full">{n.text}</p>
                    <button
                      className="ghost-btn"
                      title="Copy note to clipboard"
                      onClick={() => copyClip(n.text)}
                    >
                      ⧉ Copy
                    </button>
                  </div>
                )}
              </li>
            );
          })}
          {notes.length === 0 && (
            <li className="empty">
              {view.query
                ? `No notes matching “${view.query}”`
                : "No notes yet — try “note: buy milk” or ask the AI"}
            </li>
          )}
        </ul>
      )}

      {/* ---- Clipboard history view ---- */}
      {view.kind === "clipboard" && (
        <Fragment>
          {/* Header: clip count on the left, Clear all on the right. */}
          <div className="clip-header">
            <span className="clip-count">
              {clips.length} clip{clips.length === 1 ? "" : "s"}
            </span>
            {clips.length > 0 && (
              <button className="ghost-btn danger" onClick={clearAllClips} title="Delete all clips">
                Clear all
              </button>
            )}
          </div>
          <ul className="results" ref={listRef}>
            {clips.map((c, i) => (
              <li
                key={i}
                className={`result ${i === active ? "active" : ""}`}
                onMouseEnter={() => hover(i)}
                onClick={() => copyClip(c.text)}
              >
                <span className="badge file">Clip</span>
                <span className="result-title clip-text">{c.text.slice(0, 120)}</span>
                <span className="result-sub">{fmtDue(c.at)}</span>
              </li>
            ))}
            {clips.length === 0 && <li className="empty">Clipboard history is empty</li>}
          </ul>
        </Fragment>
      )}

      {/* ---- Dictation recordings view ---- */}
      {view.kind === "dictations" && (
        <ul className="results" ref={listRef}>
          {dictations.map((d, i) => {
            const selected = selDict === d.id;
            return (
              <li
                key={d.id}
                className={`result dictation ${i === active ? "active" : ""} ${selected ? "selected" : ""}`}
                onMouseEnter={() => hover(i)}
                onClick={() => toggleDictation(d)}
              >
                {/* One line per dictation; long transcripts are clipped. */}
                <div className="dictation-row">
                  <span className="badge mic">Mic</span>
                  <span className="result-title dictation-text">{d.text}</span>
                  <span className="result-sub">
                    {[fmtDue(d.created ?? null), fmtDur(d.duration_ms)]
                      .filter(Boolean)
                      .join(" • ")}
                  </span>
                  <button
                    className="row-trash"
                    title="Delete dictation"
                    onClick={(e) => {
                      e.stopPropagation();
                      deleteDictation(d.id);
                    }}
                  >
                    🗑
                  </button>
                </div>
                {/* Selected: full transcript + recording player. */}
                {selected && (
                  <div className="dictation-detail" onClick={(e) => e.stopPropagation()}>
                    <p className="dictation-full">{d.text}</p>
                    {d.answer && <p className="dictation-answer">{d.answer}</p>}
                    {dictAudio?.id === d.id ? (
                      <audio className="dictation-audio" controls src={dictAudio.url} />
                    ) : (
                      <span className="dictation-loading">
                        <span className="spinner" /> Loading recording…
                      </span>
                    )}
                  </div>
                )}
              </li>
            );
          })}
          {dictations.length === 0 && (
            <li className="empty">No dictations yet — hold Enter on the dictate ring</li>
          )}
        </ul>
      )}

      {/* ---- Search results / empty-state command reference ---- */}
      {view.kind === "search" && searchRows.length > 0 && (
        <ul className="results" ref={listRef}>
          {searchRows.map((r, i) => {
            // Empty query: label the sections — "TODOS" above the first
            // todo row, "COMMANDS" where the app/command rows begin.
            const header = emptyQuery
              ? i === 0
                ? r.kind === "todo"
                  ? "TODOS"
                  : "COMMANDS"
                : r.kind !== "todo" && searchRows[i - 1].kind === "todo"
                  ? "COMMANDS"
                  : null
              : null;
            return (
              <Fragment key={r.path + i}>
                {header && <li className="section-header">{header}</li>}
                <li
                  className={`result ${i === active ? "active" : ""}`}
                  onMouseEnter={() => hover(i)}
                  onClick={() => {
                    if (handleSpecial(r.path)) return;
                    if (r.path === "__ask__") sendChat(query.trim());
                    else if (r.path === "__dictate_text__") openDictate("text");
                    else if (r.path === "__dictate_supacast__") openDictate("supacast");
                    else open(r);
                  }}
                >
                  <span
                    className={`badge ${
                      r.kind === "todo"
                        ? "todo"
                        : r.kind === "app" && r.path.startsWith("__")
                          ? "ai"
                          : r.kind
                    }`}
                  >
                    {r.kind === "todo"
                      ? "Todo"
                      : r.path.startsWith("__ask__") || r.path === "__chat__"
                        ? "AI"
                        : r.kind === "app"
                          ? "App"
                          : r.kind === "folder"
                            ? "Folder"
                            : "File"}
                  </span>
                  <span className="result-title">{r.title}</span>
                  <span className="result-sub">{r.subtitle}</span>
                </li>
              </Fragment>
            );
          })}
        </ul>
      )}

      {view.kind === "search" && query.trim() && !searching && allResults.length === 0 && (
        <div className="empty">No results for “{query.trim()}”</div>
      )}
    </div>
  );
}
