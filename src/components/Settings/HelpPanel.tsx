import "./HelpPanel.css";

interface Props {
  /** Whatever the global shortcut is currently bound to, shown as typed. */
  globalHotkey: string;
  globalHotkeyEnabled: boolean;
  onClose: () => void;
}

/**
 * Help, reachable from Settings.
 *
 * Most of what this widget can do is a gesture with no affordance — drag right
 * to nest, paste a list to get a list, Shift+Enter for a description. Those are
 * quick once known and invisible until then, and a 340px note has nowhere to
 * advertise them. This is that nowhere.
 *
 * It also states what the widget deliberately does not do. A tool that is quiet
 * about its limits reads as broken when you hit one; one that names them reads
 * as considered, and saves the search for a setting that was never there.
 */
export function HelpPanel({ globalHotkey, globalHotkeyEnabled, onClose }: Props) {
  // "CmdOrCtrl" is the accelerator format Tauri wants, not something to show.
  const hotkey = globalHotkey.replace("CmdOrCtrl", "Ctrl");

  return (
    <div className="help">
      <header className="help-head" data-tauri-drag-region>
        <button
          className="icon-button"
          onClick={onClose}
          aria-label="Back to settings"
          title="Back"
        >
          {/* A real arrow, not a chevron glyph. "‹" is a typographic quote
              mark: it renders light, sits off the optical centre and changes
              shape with the font. This is the same 1.5px round-capped stroke
              as every other icon in the app. */}
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <path
              d="M12.5 8H4M7.5 4L3.5 8l4 4"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
        <span className="help-title">Help</span>
      </header>

      <div className="help-body scroll-area">
        <section className="help-section">
          <h2>Keyboard</h2>

          <h3>Anywhere</h3>
          <dl className="help-keys">
            <dt>
              <kbd>{hotkey}</kbd>
            </dt>
            <dd>
              Show or hide every note{" "}
              {globalHotkeyEnabled ? (
                <span className="help-note">— works from any app</span>
              ) : (
                <span className="help-note help-off">— currently off</span>
              )}
            </dd>
          </dl>

          <h3>In a note</h3>
          <dl className="help-keys">
            <dt>
              <kbd>Ctrl</kbd> <kbd>A</kbd>
            </dt>
            <dd>Jump to the add-task field</dd>

            <dt>
              <kbd>Ctrl</kbd> <kbd>R</kbd>
            </dt>
            <dd>Sync now</dd>

            <dt>
              <kbd>Ctrl</kbd> <kbd>,</kbd>
            </dt>
            <dd>Open or close settings</dd>

            <dt>
              <kbd>Esc</kbd>
            </dt>
            <dd>Close what is open — an editor, a menu, this page</dd>
          </dl>

          <h3>Adding tasks</h3>
          <dl className="help-keys">
            <dt>
              <kbd>Enter</kbd>
            </dt>
            <dd>
              Add the task and stay put, ready for the next one — type a whole
              list without touching the mouse
            </dd>

            <dt>
              <kbd>Shift</kbd> <kbd>Enter</kbd>
            </dt>
            <dd>
              New line. Everything after the first line becomes the task&rsquo;s
              description
            </dd>
          </dl>

          <h3>Editing a task</h3>
          <dl className="help-keys">
            <dt>
              <kbd>Enter</kbd>
            </dt>
            <dd>Save the title</dd>

            <dt>
              <kbd>Ctrl</kbd> <kbd>Enter</kbd>
            </dt>
            <dd>Save the description — plain Enter adds a line there instead</dd>

            <dt>
              <kbd>Tab</kbd>
            </dt>
            <dd>Move on: title, then description, then date</dd>

            <dt>
              <kbd>Esc</kbd>
            </dt>
            <dd>
              Cancel <span className="help-warn">and discard the edit</span> —
              clicking away saves it instead
            </dd>
          </dl>
        </section>

        <section className="help-section">
          <h2>Mouse</h2>
          <dl className="help-keys help-gestures">
            <dt>Click a task</dt>
            <dd>Opens it. Click again to close it</dd>

            <dt>Double-click a title</dt>
            <dd>Edits it, open or closed</dd>

            <dt>Click a description</dt>
            <dd>
              Edits it. One click is enough here — it is only on screen while
              the task is open
            </dd>

            <dt>Drag up or down</dt>
            <dd>Reorders</dd>

            <dt>Drag to the right</dt>
            <dd>Makes it a subtask of the task above</dd>

            <dt>Drag to the left</dt>
            <dd>Pulls a subtask back out</dd>

            <dt>Drag onto another note</dt>
            <dd>Moves it to that note&rsquo;s list</dd>

            <dt>Right-click</dt>
            <dd>Menu: date, move to another list, open in Google, delete</dd>
          </dl>
        </section>

        <section className="help-section">
          <h2>Worth knowing</h2>

          <p>
            <strong>Paste a list and you get a list.</strong> Copy several lines
            from anywhere and paste into the add field — one task per line.
            Indented lines become subtasks. Bullets and numbering are stripped.
          </p>

          <p>
            <strong>Lines beside a task mean it has a description.</strong>{" "}
            Descriptions stay hidden while a task is closed, so every row is one
            line tall — the mark is how a task with more to say says so.
          </p>

          <p>
            <strong>Delete waits.</strong> Google Tasks has no bin, so a deleted
            task is only really deleted once the Undo strip disappears. Until
            then, Undo brings it straight back.
          </p>

          <p>
            <strong>Each note is its own window.</strong> The <strong>+</strong>{" "}
            button opens another — on the same list or a different one. Its pin
            and colour belong to that note alone, and the notes you had open
            reopen next time.
          </p>

          <p>
            <strong>It is your Google Tasks.</strong> Everything here is the same
            data as the web app and your phone, synced both ways. Nothing lives
            only in this widget.
          </p>
        </section>

        <section className="help-section">
          <h2>Not included</h2>

          <dl className="help-keys help-gaps">
            <dt>Starring, repeats, attachments</dt>
            <dd>
              Google keeps these but does not offer them to apps. The repeat
              button on an open task takes you straight there; for the rest, use{" "}
              <em>Open in Google</em>
            </dd>

            <dt>Working offline</dt>
            <dd>
              Changes need a connection. A failed one is reported and undone
              rather than queued
            </dd>

            <dt>Several lists in one note</dt>
            <dd>One note shows one list. Open another note for another list</dd>

            <dt>Selecting several tasks</dt>
            <dd>One at a time for now</dd>

            <dt>Keyboard reordering</dt>
            <dd>Moving and nesting are drag-only</dd>

            <dt>Filters and search</dt>
            <dd>Not on a list this size</dd>
          </dl>
        </section>

        <section className="help-section">
          <h2>Maybe later</h2>
          <ul className="help-future">
            <li>Arrow keys through tasks, and nesting without the mouse</li>
            <li>Select several tasks to complete, move or delete together</li>
            <li>Copy tasks back out as indented text</li>
            <li>Queue changes made offline and send them when you reconnect</li>
            <li>A countdown on the Undo strip, so its window is visible</li>
          </ul>
          <p className="help-note">
            Nothing here is promised — it is what the shape of the app suggests
            next.
          </p>
        </section>
      </div>
    </div>
  );
}
