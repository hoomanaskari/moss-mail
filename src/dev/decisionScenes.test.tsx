import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { useStore } from "../state/store";
import { resetDemoStore } from "../test/fixtures";
import { decisionScenes } from "./decisionScenes";

beforeEach(() => resetDemoStore());
afterEach(() => history.replaceState(null, "", "/"));

// Each state renders in both themes (App.test checks every scene in light).
describe.each(["dark", "light"])("decision scenes in %s", (theme) => {
  it.each(Object.keys(decisionScenes))("%s", async (scene) => {
    history.replaceState(null, "", `/?scene=${scene}&theme=${theme}`);
    render(<App />);
    await act(() => import("./scenes"));
    expect(screen.queryByText(/Something went wrong on this screen/)).not.toBeInTheDocument();
    expect((await screen.findAllByText(decisionScenes[scene].shows)).length).toBeGreaterThan(0);
    expect(useStore.getState().theme).toBe(theme);
  });
});

it("each scene can run twice without doubling the waiting email", () => {
  decisionScenes["inbox-decided"].apply();
  decisionScenes["inbox-decided"].apply();
  expect(useStore.getState().threads.filter((t) => t.id === "testing-plan")).toHaveLength(1);
});
