import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn().mockResolvedValue(undefined),
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
}));

import { TaskWidget } from "./TaskWidget";
import type { Task } from "../../types";

type Props = React.ComponentProps<typeof TaskWidget>;

const task = (id: string, title: string, parentId: string | null = null): Task => ({
  id,
  title,
  parentId,
  notes: null,
  due: null,
  status: "needsAction",
  position: id,
  updated: "2026-01-01T00:00:00.000Z",
});

const tasks = [
  task("a", "Alpha"),
  task("a1", "Alpha child", "a"),
  task("b", "Bravo"),
  task("c", "Charlie"),
];

let props: Props;

beforeEach(() => {
  const noop = vi.fn();
  const noopAsync = vi.fn().mockResolvedValue(null);
  props = {
    tasks,
    taskLists: [{ id: "list-1", title: "List" }],
    settings: {
      windowLayer: "normal",
      showInTaskbar: true,
      startWithWindows: false,
      startHidden: false,
      globalHotkey: "CmdOrCtrl+Shift+G",
      globalHotkeyEnabled: true,
      theme: "system",
      selectedTaskListId: "list-1",
    },
    status: { state: "synced", lastSyncedAt: null, message: null },
    settlingIds: new Set(),
    errors: {},
    color: null,
    widgetError: null,
    pendingDelete: null,
    onToggle: vi.fn(),
    onDelete: noop,
    onDeleteMany: vi.fn(),
    onEdit: noop,
    onSetDue: noop,
    onOpenInGoogle: noop,
    onMoveToList: noop,
    onMoveTo: noop,
    onDepart: noop,
    onAdopt: noop,
    onCreateList: noopAsync,
    onRenameList: noopAsync,
    onDeleteList: noopAsync,
    onAdd: noopAsync,
    onAddSubtask: noopAsync,
    onAddOutline: noopAsync,
    onSelectList: noop,
    onDuplicateNote: noop,
    onChangeLayer: noop,
    onChangeColor: noop,
    onUndoDelete: noop,
    onSyncNow: noop,
    onOpenSettings: noop,
    onOpenHelp: noop,
    onHide: noop,
    onQuit: vi.fn().mockResolvedValue(undefined),
  } as Props;
});

function selectBravoAndCharlie() {
  render(<TaskWidget {...props} />);
  fireEvent.click(screen.getByText("Bravo"), { ctrlKey: true, detail: 1 });
  fireEvent.click(screen.getByText("Charlie"), { ctrlKey: true, detail: 1 });
}

describe("TaskWidget group delete", () => {
  it("asks before deleting a selection with the Delete key", () => {
    selectBravoAndCharlie();
    fireEvent.keyDown(document.body, { key: "Delete" });

    expect(screen.getByRole("alert")).toHaveTextContent("Delete 2 tasks?");
    // Asking is all that has happened.
    expect(props.onDeleteMany).not.toHaveBeenCalled();
  });

  it("deletes the selection once confirmed with Enter", () => {
    selectBravoAndCharlie();
    fireEvent.keyDown(document.body, { key: "Delete" });
    fireEvent.keyDown(document.body, { key: "Enter" });

    expect(props.onDeleteMany).toHaveBeenCalledTimes(1);
    expect(props.onDeleteMany).toHaveBeenCalledWith(["b", "c"]);
  });

  it("deletes once confirmed with the button", () => {
    selectBravoAndCharlie();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    expect(props.onDeleteMany).toHaveBeenCalledWith(["b", "c"]);
  });

  // Backing out of the question is not the same as abandoning the selection.
  it("keeps the selection when Escape cancels the question", () => {
    selectBravoAndCharlie();
    fireEvent.keyDown(document.body, { key: "Delete" });
    fireEvent.keyDown(document.body, { key: "Escape" });

    expect(props.onDeleteMany).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByText("2 selected")).toBeInTheDocument();
  });

  it("names the subtasks that go with a selected parent", () => {
    render(<TaskWidget {...props} />);
    fireEvent.click(screen.getByText("Alpha"), { ctrlKey: true, detail: 1 });
    fireEvent.keyDown(document.body, { key: "Delete" });

    expect(screen.getByRole("alert")).toHaveTextContent("Delete 1 task and 1 subtask?");
  });
});

describe("TaskWidget multi-tick", () => {
  it("completes the whole selection from any selected checkbox", () => {
    selectBravoAndCharlie();
    const bravoCheckbox = screen.getAllByRole("checkbox")[2];
    fireEvent.click(bravoCheckbox);

    expect(props.onToggle).toHaveBeenCalledTimes(2);
    expect(props.onToggle).toHaveBeenCalledWith("b");
    expect(props.onToggle).toHaveBeenCalledWith("c");
  });

  it("completes just that task from a checkbox outside the selection", () => {
    selectBravoAndCharlie();
    const alphaCheckbox = screen.getAllByRole("checkbox")[0];
    fireEvent.click(alphaCheckbox);

    expect(props.onToggle).toHaveBeenCalledTimes(1);
    expect(props.onToggle).toHaveBeenCalledWith("a");
  });
});
