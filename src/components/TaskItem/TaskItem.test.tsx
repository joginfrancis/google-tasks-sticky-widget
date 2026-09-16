import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// src/lib/window.ts caches the window label on first call, so Tauri has to be
// mocked at module level rather than inside a test.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn().mockResolvedValue(undefined),
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
}));

import { TaskItem } from "./TaskItem";
import type { Task } from "../../types";

const task: Task = {
  id: "task-1",
  parentId: null,
  title: "Water the plants",
  notes: "The big one by the window",
  due: null,
  status: "needsAction",
  position: "00000000000000000000",
  updated: "2026-01-01T00:00:00.000Z",
};

let handlers: {
  onToggle: ReturnType<typeof vi.fn>;
  onDelete: ReturnType<typeof vi.fn>;
  onEdit: ReturnType<typeof vi.fn>;
  onSetDue: ReturnType<typeof vi.fn>;
  onOpenInGoogle: ReturnType<typeof vi.fn>;
  onMoveToList: ReturnType<typeof vi.fn>;
  onToggleExpand: ReturnType<typeof vi.fn>;
};

function setup(overrides: Partial<React.ComponentProps<typeof TaskItem>> = {}) {
  const props = {
    task,
    isSubtask: false,
    isSettling: false,
    error: null,
    otherLists: [],
    isExpanded: false,
    ...handlers,
    ...overrides,
  } as React.ComponentProps<typeof TaskItem>;
  // Rendered into a <ul>: TaskItem is an <li>, and jsdom is happier with a
  // valid parent than with the warning.
  return render(<TaskItem {...props} />, {
    container: document.body.appendChild(document.createElement("ul")),
  });
}

/**
 * The row holds a single click for 220ms to see whether a second one follows,
 * so every expand assertion has to step past that timer deliberately.
 */
const GRACE_MS = 220;

function runGrace() {
  act(() => {
    vi.advanceTimersByTime(GRACE_MS + 10);
  });
}

const title = () => screen.getByText("Water the plants");

beforeEach(() => {
  handlers = {
    onToggle: vi.fn(),
    onDelete: vi.fn(),
    onEdit: vi.fn(),
    onSetDue: vi.fn(),
    onOpenInGoogle: vi.fn(),
    onMoveToList: vi.fn(),
    onToggleExpand: vi.fn(),
  };
});

describe("TaskItem expansion", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("toggles expansion on a single click, but only after the double-click grace", () => {
    setup();
    fireEvent.click(title(), { detail: 1 });

    // Nothing yet: the row is still waiting to see if a second click arrives.
    expect(handlers.onToggleExpand).not.toHaveBeenCalled();

    runGrace();
    expect(handlers.onToggleExpand).toHaveBeenCalledWith("task-1");
  });

  it("enters edit mode on a double click without ever expanding", () => {
    // Without the grace period every rename would flash the row open and shut.
    setup();
    const node = title();
    fireEvent.click(node, { detail: 1 });
    fireEvent.click(node, { detail: 2 });
    fireEvent.doubleClick(node);

    runGrace();
    expect(handlers.onToggleExpand).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox")).toHaveValue("Water the plants");
  });

  it("does not toggle expansion when the checkbox is clicked", () => {
    // Pressing a control should do one thing, not two.
    setup();
    fireEvent.click(screen.getByRole("checkbox"));
    runGrace();

    expect(handlers.onToggle).toHaveBeenCalledWith("task-1");
    expect(handlers.onToggleExpand).not.toHaveBeenCalled();
  });

  it("does not toggle expansion when the options button is clicked", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: "Task options" }));
    runGrace();

    expect(handlers.onToggleExpand).not.toHaveBeenCalled();
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("does not treat the click that ends a drag as an expand", () => {
    // A pointer that moved more than a few pixels was a drag, and the click
    // browsers fire at the end of it is not a request to open the row.
    setup({ onDragPress: vi.fn() });
    const node = title();
    fireEvent.pointerDown(node, { clientX: 10, clientY: 10 });
    fireEvent.click(node, { detail: 1, clientX: 60, clientY: 44 });

    runGrace();
    expect(handlers.onToggleExpand).not.toHaveBeenCalled();
  });
});

describe("TaskItem notes visibility", () => {
  it("hides notes while collapsed so every row is one line tall", () => {
    setup();
    expect(screen.queryByText("The big one by the window")).not.toBeInTheDocument();
  });

  it("shows notes when expanded, with a placeholder when there are none", () => {
    setup({ isExpanded: true });
    expect(screen.getByText("The big one by the window")).toBeInTheDocument();

    // The placeholder is the one thing a collapsed row cannot express.
    cleanupAndRender();
    expect(screen.getByText("Add details…")).toBeInTheDocument();
  });

  // Hiding the notes is what makes every row one line tall; the mark is what
  // stops that hiding them completely. The pair only works together.
  it("marks a collapsed row that has a description", () => {
    setup();
    expect(screen.getByTitle("Has details")).toBeInTheDocument();
  });

  it("does not mark a collapsed row with no description", () => {
    setup({ task: { ...task, notes: null } });
    expect(screen.queryByTitle("Has details")).not.toBeInTheDocument();
  });

  it("drops the mark once open, where the description speaks for itself", () => {
    setup({ isExpanded: true });
    expect(screen.queryByTitle("Has details")).not.toBeInTheDocument();
  });

  function cleanupAndRender() {
    document.body.innerHTML = "";
    setup({ isExpanded: true, task: { ...task, notes: null } });
  }
});

describe("TaskItem editing", () => {
  it("commits a title edit on Enter", async () => {
    const user = userEvent.setup();
    setup();
    await user.dblClick(title());

    const editor = screen.getByRole("textbox");
    await user.clear(editor);
    await user.type(editor, "Water the ferns{Enter}");

    expect(handlers.onEdit).toHaveBeenCalledWith("task-1", { title: "Water the ferns" });
    // The editor closes on commit. The displayed title still reads the old
    // value because `task` is owned by the list — the row reports the edit
    // upward and waits to be re-rendered rather than keeping its own copy.
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("does not save an unchanged title", async () => {
    const user = userEvent.setup();
    setup();
    await user.dblClick(title());
    await user.keyboard("{Enter}");

    expect(handlers.onEdit).not.toHaveBeenCalled();
  });

  it("treats an emptied title as a cancel rather than an error", async () => {
    // The API would reject it anyway; surfacing a failure the user did not
    // intend is worse than quietly keeping the old title.
    const user = userEvent.setup();
    setup();
    await user.dblClick(title());
    await user.clear(screen.getByRole("textbox"));
    await user.keyboard("{Enter}");

    expect(handlers.onEdit).not.toHaveBeenCalled();
  });

  it("cancels an edit on Escape without saving", async () => {
    const user = userEvent.setup();
    setup();
    await user.dblClick(title());
    await user.type(screen.getByRole("textbox"), " and the ferns");
    await user.keyboard("{Escape}");

    expect(handlers.onEdit).not.toHaveBeenCalled();
    expect(screen.getByText("Water the plants")).toBeInTheDocument();
  });

  it("inserts a newline on Enter in the notes editor and commits on Ctrl+Enter", async () => {
    // A description is prose: breaking a line is the common intent there, the
    // opposite of a title where Enter means "done".
    const user = userEvent.setup();
    setup({ isExpanded: true });
    // The description opens on a single click now; only the title needs two.
    await user.click(screen.getByText("The big one by the window"));

    const editor = screen.getByRole("textbox") as HTMLTextAreaElement;
    await user.clear(editor);
    await user.type(editor, "First{Enter}Second");

    expect(handlers.onEdit).not.toHaveBeenCalled();
    expect(editor.value).toBe("First\nSecond");

    await user.keyboard("{Control>}{Enter}{/Control}");
    expect(handlers.onEdit).toHaveBeenCalledWith("task-1", { notes: "First\nSecond" });
  });

  it("walks Tab from the title editor into the notes editor, committing on the way", async () => {
    const user = userEvent.setup();
    setup({ isExpanded: true });
    await user.dblClick(title());

    const titleEditor = screen.getByRole("textbox");
    await user.clear(titleEditor);
    await user.type(titleEditor, "Renamed");
    await user.tab();

    // The title edit was saved, not dropped, before moving on.
    expect(handlers.onEdit).toHaveBeenCalledWith("task-1", { title: "Renamed" });
    // And the notes editor is now the one open, seeded with the existing notes.
    expect(screen.getByRole("textbox")).toHaveValue("The big one by the window");
  });

  it("walks Shift+Tab from the notes editor back to the title editor", async () => {
    const user = userEvent.setup();
    setup({ isExpanded: true });
    // The description opens on a single click now; only the title needs two.
    await user.click(screen.getByText("The big one by the window"));

    await user.tab({ shift: true });
    expect(screen.getByRole("textbox")).toHaveValue("Water the plants");
  });

  it("refuses to edit a completed task, since that is nearly always a misclick", async () => {
    const user = userEvent.setup();
    setup({ task: { ...task, status: "completed" } });
    await user.dblClick(title());

    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });
});
