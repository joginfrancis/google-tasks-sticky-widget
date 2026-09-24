import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// The component tree reaches Tauri through src/lib/window.ts, which caches the
// window label on first call — so the mock has to be in place at module load,
// not inside a test body.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn().mockResolvedValue(undefined),
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
}));

import { TaskInput } from "./TaskInput";

/** Resolving null is the component's contract for "the write succeeded". */
type Props = React.ComponentProps<typeof TaskInput>;
let onSubmit: Mock<Props["onSubmit"]>;
let onSubmitOutline: Mock<Props["onSubmitOutline"]>;

function setup() {
  return render(<TaskInput onSubmit={onSubmit} onSubmitOutline={onSubmitOutline} />);
}

/** Opens the collapsed quick-add and returns the textarea. */
async function open(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: /add task/i }));
  return screen.getByPlaceholderText("What needs doing?") as HTMLTextAreaElement;
}

beforeEach(() => {
  onSubmit = vi.fn().mockResolvedValue(null);
  onSubmitOutline = vi.fn().mockResolvedValue(null);
});

describe("TaskInput", () => {
  it("commits on Enter and stays open, empty and focused for the next task", async () => {
    // The whole point of the quick-add: typing a list should never require a
    // reach for the mouse between entries.
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.type(input, "Buy milk{Enter}");

    expect(onSubmit).toHaveBeenCalledWith("Buy milk", undefined);
    await waitFor(() => expect(input).toHaveValue(""));
    expect(input).toHaveFocus();
  });

  it("inserts a newline on Shift+Enter instead of committing", async () => {
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.type(input, "Line one{Shift>}{Enter}{/Shift}Line two");

    expect(onSubmit).not.toHaveBeenCalled();
    expect(input.value).toBe("Line one\nLine two");
  });

  it("treats the first line as the title and everything after it as notes", async () => {
    // A title is a label shown in tight places; a two-line one reads badly, so
    // the break is the boundary between title and description.
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.type(
      input,
      "Call the bank{Shift>}{Enter}{/Shift}Ask about the fee{Shift>}{Enter}{/Shift}and the limit{Enter}",
    );

    expect(onSubmit).toHaveBeenCalledWith("Call the bank", "Ask about the fee\nand the limit");
  });

  it("clears and closes on Escape", async () => {
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.type(input, "never mind");
    await user.keyboard("{Escape}");

    expect(onSubmit).not.toHaveBeenCalled();
    // Back to the collapsed trigger, with nothing left behind.
    expect(screen.getByRole("button", { name: /add task/i })).toBeInTheDocument();
    expect(screen.queryByPlaceholderText("What needs doing?")).not.toBeInTheDocument();
  });

  it("puts the full typed text back and shows the error when the write fails (SPEC 3.3)", async () => {
    // The field is cleared optimistically so a fast success feels instant; a
    // failure must not turn that optimism into lost typing.
    onSubmit.mockResolvedValue("Offline — could not reach Google");
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.type(input, "Draft the memo{Shift>}{Enter}{/Shift}with the Q3 numbers{Enter}");

    await waitFor(() =>
      expect(screen.getByText("Offline — could not reach Google")).toBeInTheDocument(),
    );
    // Restored in full — including the notes lines, not just the title.
    expect(input).toHaveValue("Draft the memo\nwith the Q3 numbers");
  });

  it("clears the error as soon as the user types again", async () => {
    onSubmit.mockResolvedValue("Nope");
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.type(input, "thing{Enter}");
    await waitFor(() => expect(screen.getByText("Nope")).toBeInTheDocument());

    await user.type(input, "!");
    expect(screen.queryByText("Nope")).not.toBeInTheDocument();
  });

  it("turns a multi-line paste into an outline and creates no single task", async () => {
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.click(input);
    await user.paste("Groceries\n  Milk\n  Bread\nCall mum");

    await waitFor(() => expect(onSubmitOutline).toHaveBeenCalledTimes(1));
    expect(onSubmitOutline).toHaveBeenCalledWith([
      { title: "Groceries", depth: 0 },
      { title: "Milk", depth: 1 },
      { title: "Bread", depth: 1 },
      { title: "Call mum", depth: 0 },
    ]);
    // The paste path replaces the normal one; the text must not also land as a
    // single task with the rest as notes.
    expect(onSubmit).not.toHaveBeenCalled();
    await waitFor(() => expect(input).toHaveValue(""));
  });

  it("leaves a single-line paste alone so it can be edited before committing", async () => {
    // Hijacking a one-line paste would make it impossible to paste a title in.
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.click(input);
    await user.paste("Renew the passport");

    expect(onSubmitOutline).not.toHaveBeenCalled();
    expect(input).toHaveValue("Renew the passport");

    // …and it still commits as one ordinary task.
    await user.type(input, "{Enter}");
    expect(onSubmit).toHaveBeenCalledWith("Renew the passport", undefined);
  });

  it("shows how many tasks a paste is creating while it runs", async () => {
    let finish: (value: string | null) => void = () => {};
    onSubmitOutline.mockReturnValue(
      new Promise<string | null>((resolve) => {
        finish = resolve;
      }),
    );
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.click(input);
    await user.paste("One\nTwo\nThree");

    expect(await screen.findByRole("status")).toHaveTextContent("Adding 3 tasks…");
    finish(null);
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
  });

  it("stays open on an empty Enter, ready for the next task", async () => {
    // Enter again after adding a task means "next", not "done". Closing here
    // turned typing a list into type, Enter, click back in, repeat.
    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.type(input, "   {Enter}");

    expect(onSubmit).not.toHaveBeenCalled();
    expect(input).toBeInTheDocument();
    expect(input).toHaveFocus();
  });

  it("keeps the field open and focused across a slow save, for a whole list", async () => {
    // The bug this guards: the field was disabled while a save was in flight.
    // Disabling a focused field throws focus out, which ran onBlur against the
    // just-cleared value and closed the box after every Enter. An instantly
    // resolving save never held the field disabled long enough to show it, so
    // this one resolves only when told to.
    let finishSave: (value: string | null) => void = () => {};
    onSubmit = vi.fn(
      () => new Promise<string | null>((resolve) => (finishSave = resolve)),
    );

    const user = userEvent.setup();
    setup();
    const input = await open(user);

    await user.type(input, "Buy milk{Enter}");
    // Still saving. The field must not be disabled: in a real webview that
    // throws focus out and closes the box. jsdom does not move focus off a
    // disabled field, so checking focus alone passed with the bug in place —
    // this assertion is the one that actually guards it.
    expect(input).not.toBeDisabled();
    expect(input).toBeInTheDocument();
    expect(input).toHaveFocus();

    finishSave(null);
    await waitFor(() => expect(input).toHaveValue(""));

    // An extra Enter in between, as someone typing quickly would press.
    await user.type(input, "{Enter}Buy eggs{Enter}");
    finishSave(null);

    expect(onSubmit).toHaveBeenCalledTimes(2);
    expect(onSubmit).toHaveBeenLastCalledWith("Buy eggs", undefined);
    await waitFor(() => expect(input).toHaveValue(""));
    expect(input).toHaveFocus();
  });
});

describe("TaskInput Ctrl+A", () => {
  it("leaves Ctrl+A alone while a description is being edited", () => {
    const noop = vi.fn().mockResolvedValue(null);
    render(<TaskInput onSubmit={noop} onSubmitOutline={noop} />);

    // A description editor is a contenteditable, not a textarea — the shortcut
    // used to miss that and jump out of the description mid-edit.
    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "true");
    document.body.appendChild(editor);
    editor.focus();

    fireEvent.keyDown(editor, { key: "a", ctrlKey: true });
    expect(screen.queryByPlaceholderText("What needs doing?")).toBeNull();

    // With focus outside any text, it still opens the add field.
    fireEvent.keyDown(document.body, { key: "a", ctrlKey: true });
    expect(screen.getByPlaceholderText("What needs doing?")).toBeInTheDocument();

    editor.remove();
  });
});
