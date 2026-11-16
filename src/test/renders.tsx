import { act, render } from "@testing-library/react";
import { Profiler, type ReactElement } from "react";
import { vi } from "vitest";
import { useStore } from "../state/store";

/** Renders `ui` and counts its commits after the first, so a test can see what redraws it. */
export function renderCounted(ui: ReactElement) {
  const renders = vi.fn();
  const view = render(
    <Profiler id="counted" onRender={renders}>
      {ui}
    </Profiler>,
  );
  renders.mockClear();
  return { ...view, renders };
}

/** Store changes no list screen shows: a typed Quick reply, the palette's text, a file's thumbnail. */
export function changeWhatNoListShows() {
  act(() => useStore.setState({ paletteQuery: "budget", replyDraft: "Thanks", previews: { "m:0": null } }));
}
