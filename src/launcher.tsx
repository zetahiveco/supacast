import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface SearchResult {
  title: string;
  subtitle: string;
  path: string;
  kind: "app" | "file";
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
    if (chat.length > 0) return; // in chat, typing edits the next message
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
  }, [query, chat.length]);

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
    } else if (view.kind === "clipboard") {
      invoke<ClipEntry[]>("get_clipboard_history").then(setClips).catch(console.error);
      setActive(0);
    }
  }, [view]);

  // Keep the chat scrolled to the newest message.
  useEffect(() => {
    chatScrollRef.current?.scrollTo({ top: chatScrollRef.current.scrollHeight });
  }, [chat]);

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

  const toggleTodo = useCallback(async (todo: Todo) => {
    // Check ⇄ uncheck (keeps the todo; unchecking also re-arms reminders).
    await invoke("set_todo_done", { id: todo.id, done: !todo.done });
    invoke<Todo[]>("list_todos", { scope: "all" }).then(setTodos).catch(console.error);
  }, []);

  const deleteTodo = useCallback(async (id: string) => {
    await invoke("delete_todo", { id });
    invoke<Todo[]>("list_todos", { scope: "all" }).then(setTodos).catch(console.error);
  }, []);

  const deleteNote = useCallback(
    async (id: string) => {
      await invoke("delete_note", { id });
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
        } else {
          sendChat(query);
        }
      }
      return;
    }

    if (e.key === "ArrowDown") {
      e.preventDefault();
      mouseMovedRef.current = false; // keyboard selection wins over hover
      const len =
        view.kind === "search"
          ? allResults.length
          : view.kind === "todos"
            ? flatTodos.length
            : view.kind === "notes"
              ? notes.length
              : clips.length;
      setActive((a) => Math.min(a + 1, len - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      mouseMovedRef.current = false;
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (view.kind === "search") {
        const item = allResults[active];
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
        // Enter copies the note to the clipboard so it can be pasted anywhere.
        copyClip(notes[active].text);
      } else if (view.kind === "clipboard" && clips[active]) {
        copyClip(clips[active].text);
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
  }
  const allResults = [...suggestions, ...results];

  const isDictateQuery = /^dictate\s*\(/i.test(query.trim());

  return (
    <div className="launcher">
      <div className="search-row">
        <span className="rocket">🚀</span>
        <input
          ref={inputRef}
          className="search-input"
          placeholder={
            view.kind === "chat" && chat.length > 0
              ? "Reply to Supacast…"
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

      {view.kind === "chat" && chat.length === 0 && isDictateQuery && (
        <div className="agent-view">
          <p className="agent-hint">
            Press Enter to start {/text/i.test(query) ? "Dictate (Text)" : "Dictate (Supacast)"}…
          </p>
        </div>
      )}

      {/* ---- Todos view (date-wise, checkable) ---- */}
      {view.kind === "todos" && (
        <ul className="results">
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
        <ul className="results">
          {notes.map((n, i) => (
            <li
              key={n.id}
              className={`result ${i === active ? "active" : ""}`}
              onMouseEnter={() => hover(i)}
              onClick={() => copyClip(n.text)}
            >
              <span className="badge note">Note</span>
              <span className="result-title clip-text">{n.text}</span>
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
            </li>
          ))}
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
        <ul className="results">
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
      )}

      {/* ---- Search results ---- */}
      {view.kind === "search" && allResults.length > 0 && (
        <ul className="results">
          {allResults.map((r, i) => (
            <li
              key={r.path + i}
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
              <span className={`badge ${r.kind === "app" && r.path.startsWith("__") ? "ai" : r.kind}`}>
                {r.path.startsWith("__ask__") ? "AI" : r.kind === "app" ? "App" : "File"}
              </span>
              <span className="result-title">{r.title}</span>
              <span className="result-sub">{r.subtitle}</span>
            </li>
          ))}
        </ul>
      )}

      {view.kind === "search" && query.trim() && !searching && allResults.length === 0 && (
        <div className="empty">No results for “{query.trim()}”</div>
      )}
      {view.kind === "search" && !query.trim() && (
        <div className="empty hint">
          Type to search • Try “todo”, “notes”, “clipboard history”, “dictate”, “note: …”
        </div>
      )}
    </div>
  );
}
