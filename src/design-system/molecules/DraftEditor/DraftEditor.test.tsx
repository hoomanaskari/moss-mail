import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Editor } from "@tiptap/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DraftEditor } from "./DraftEditor";
import styles from "./DraftEditor.module.css";

const body = () => screen.findByRole("textbox", { name: "Email body" });
/** TipTap hangs the editor instance on its DOM node. */
const editorOf = (box: HTMLElement) => (box as HTMLElement & { editor: Editor }).editor;
/** The fake caret: the frame's only direct child besides the editor wrapper. */
const caret = (container: HTMLElement) => container.firstElementChild!.querySelector(":scope > span");

afterEach(() => {
  vi.useRealTimers();
});

describe("DraftEditor", () => {
  it("turns plain text into paragraphs and line breaks, escaping HTML", async () => {
    render(<DraftEditor initial={"Hi Lena,\n\nNumbers attached.\nThanks <3 & bye"} onChange={() => {}} />);
    const box = await body();
    expect(box).toHaveAttribute("aria-multiline", "true");
    const paragraphs = box.querySelectorAll("p");
    expect(paragraphs).toHaveLength(2);
    expect(paragraphs[0]).toHaveTextContent("Hi Lena,");
    expect(paragraphs[1].innerHTML).toBe("Numbers attached.<br>Thanks &lt;3 &amp; bye");
  });

  it("shows the default placeholder when empty", async () => {
    const box = await (render(<DraftEditor initial="" onChange={() => {}} />), body());
    expect(box.querySelector("p")).toHaveAttribute("data-placeholder", "Write your email");
  });

  it("shows a custom placeholder", async () => {
    render(<DraftEditor initial="" placeholder="Say thanks" onChange={() => {}} />);
    expect((await body()).querySelector("p")).toHaveAttribute("data-placeholder", "Say thanks");
  });

  it("reports plain text and HTML as you type", async () => {
    const onChange = vi.fn();
    render(<DraftEditor initial="" autoFocus onChange={onChange} />);
    const box = await body();
    const user = userEvent.setup();
    await user.click(box);
    await user.keyboard("Hi");
    expect(onChange).toHaveBeenLastCalledWith("Hi", "<p>Hi</p>");
    await user.keyboard("{Enter}there");
    expect(onChange).toHaveBeenLastCalledWith("Hi\n\nthere", "<p>Hi</p><p>there</p>");
  });

  it("⌘↵ and Ctrl+↵ don't insert a line break; plain Enter does", async () => {
    const onChange = vi.fn();
    render(<DraftEditor initial="Hi" autoFocus onChange={onChange} />);
    const box = await body();
    const user = userEvent.setup();
    await user.click(box);
    await user.keyboard("{Meta>}{Enter}{/Meta}{Control>}{Enter}{/Control}");
    expect(onChange).not.toHaveBeenCalled();
    expect(box.querySelectorAll("p")).toHaveLength(1);
    await user.keyboard("{Enter}");
    expect(box.querySelectorAll("p")).toHaveLength(2);
  });

  it("autoFocus puts the cursor at the end and draws the caret", async () => {
    const { container } = render(<DraftEditor initial="Hi" autoFocus onChange={() => {}} />);
    const box = await body();
    await waitFor(() => expect(box).toHaveFocus());
    const editor = editorOf(box);
    expect(editor.state.selection.head).toBe(editor.state.doc.content.size - 1);
    const drawn = caret(container) as HTMLElement;
    expect(drawn).toHaveAttribute("aria-hidden", "true");
    expect(drawn.style.height).toBe("22px");
  });

  it("without autoFocus there is no caret until the body is focused, and it hides on blur", async () => {
    const { container } = render(<DraftEditor initial="Hi" onChange={() => {}} />);
    const box = await body();
    expect(box).not.toHaveFocus();
    expect(caret(container)).toBeNull();
    act(() => {
      editorOf(box).commands.focus("end");
    });
    await waitFor(() => expect(caret(container)).not.toBeNull());
    act(() => {
      box.blur();
      fireEvent.blur(box);
    });
    expect(caret(container)).toBeNull();
  });

  it("holds the caret solid while moving and blinks once it rests", async () => {
    const { container } = render(<DraftEditor initial="Hi" autoFocus onChange={() => {}} />);
    const box = await body();
    await waitFor(() => expect(caret(container)).not.toBeNull());
    vi.useFakeTimers();
    act(() => {
      editorOf(box).commands.setTextSelection(1);
    });
    expect(caret(container)).toBeInTheDocument();
    expect(caret(container)).not.toHaveClass(styles.blink);
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(caret(container)).toHaveClass(styles.blink);
  });

  it("hides the caret while text is selected", async () => {
    const { container } = render(<DraftEditor initial="Hello" autoFocus onChange={() => {}} />);
    const box = await body();
    await waitFor(() => expect(caret(container)).not.toBeNull());
    act(() => {
      editorOf(box).commands.selectAll();
    });
    expect(caret(container)).toBeNull();
  });

  it("draws no caret when the position can't be measured", async () => {
    const { container } = render(<DraftEditor initial="Hello" autoFocus onChange={() => {}} />);
    const box = await body();
    await waitFor(() => expect(caret(container)).not.toBeNull());
    const editor = editorOf(box);
    vi.spyOn(editor.view, "coordsAtPos").mockImplementation(() => {
      throw new Error("no layout");
    });
    act(() => {
      editor.commands.setTextSelection(2);
    });
    expect(caret(container)).toBeNull();
  });

  it("stops listening to the editor when unmounted", async () => {
    const { unmount } = render(<DraftEditor initial="Hi" autoFocus onChange={() => {}} />);
    const editor = editorOf(await body());
    const off = vi.spyOn(editor, "off");
    unmount();
    expect(off.mock.calls.map(([event]) => event)).toEqual(
      expect.arrayContaining(["selectionUpdate", "update", "focus", "blur"]),
    );
  });

  it("re-places the caret when the frame resizes, and ignores resizes after unmount", async () => {
    let onResize: () => void = () => {};
    const observed: Element[] = [];
    const disconnect = vi.fn();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(cb: () => void) {
          onResize = cb;
        }
        observe(el: Element) {
          observed.push(el);
        }
        unobserve() {}
        disconnect = disconnect;
      },
    );
    try {
      const { container, unmount } = render(<DraftEditor initial="Hello" autoFocus onChange={() => {}} />);
      const box = await body();
      expect(observed).toEqual([container.firstElementChild]);
      await waitFor(() => expect(caret(container)).not.toBeNull());
      const editor = editorOf(box);
      const coords = vi.spyOn(editor.view, "coordsAtPos");
      act(() => onResize());
      expect(coords).toHaveBeenCalled();
      unmount();
      expect(disconnect).toHaveBeenCalled();
      coords.mockClear();
      act(() => onResize());
      expect(coords).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
