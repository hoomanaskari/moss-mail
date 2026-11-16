import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "../../state/store";
import { resetDemoStore } from "../../test/fixtures";
import { AppearanceSection } from "./AppearanceSection";
import { changeWhatNoListShows, renderCounted } from "../../test/renders";

const s = () => useStore.getState();

beforeEach(() => {
  resetDemoStore();
  s().openSettings("appearance");
});

describe("Appearance settings", () => {
  it("shows the current theme and reading size, and no font choice (Figtree is the only font)", () => {
    render(<AppearanceSection />);
    expect(screen.getByRole("radio", { name: /System/ })).toBeChecked();
    expect(screen.queryByRole("radiogroup", { name: "Font" })).not.toBeInTheDocument();
    expect(screen.queryByText(/Atkinson/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Figtree/)).not.toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Default" })).toHaveAttribute("aria-checked", "true");
  });

  it("changes each setting by clicking", async () => {
    const user = userEvent.setup();
    render(<AppearanceSection />);
    await user.click(screen.getByText("Warm paper"));
    expect(s()).toMatchObject({ themeChoice: "light", theme: "light" });
    await user.click(screen.getByRole("radio", { name: "Larger" }));
    expect(s().readingSize).toBe("larger");
  });

  it("← → change the theme when nothing is focused, and the size inside Reading size", async () => {
    const user = userEvent.setup();
    render(<AppearanceSection />);
    await user.keyboard("{ArrowRight}");
    expect(s().themeChoice).toBe("light");
    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(s().themeChoice).toBe("dark");
    screen.getByRole("radio", { name: "Default" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(s().readingSize).toBe("larger");
    await user.keyboard("{ArrowRight}");
    expect(s().readingSize).toBe("smaller");
    expect(s().themeChoice).toBe("dark");
  });

  it("leaves other keys and focused radios alone", async () => {
    const user = userEvent.setup();
    render(<AppearanceSection />);
    await user.keyboard("j{Alt>}{ArrowRight}{/Alt}");
    expect(s().themeChoice).toBe("system");
    screen.getByRole("radio", { name: /Light/ }).focus();
    await user.keyboard("{ArrowRight}");
    // The browser moves between native radios itself (Light → Dark); our handler doesn't also step the theme.
    expect(s().themeChoice).toBe("dark");
  });
});

it("a store change it doesn't show doesn't redraw it", () => {
  const { renders } = renderCounted(<AppearanceSection />);
  changeWhatNoListShows();
  expect(renders).not.toHaveBeenCalled();
});
