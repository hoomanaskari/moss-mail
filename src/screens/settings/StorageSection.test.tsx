import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { demoAi, setAi } from "../../services/ai";
import { demoStorage, type StorageInfo } from "../../services/mail";
import { fakeMail } from "../../test/fakeMail";
import { StorageSection } from "./StorageSection";

async function renderWith(info: Partial<StorageInfo> = {}) {
  const shown = { ...demoStorage, ...info };
  const mail = fakeMail({ storage: vi.fn(async () => shown) });
  render(<StorageSection />);
  await screen.findByText("142 MB of 500 MB");
  return { ...mail, shown };
}

it("shows where the space goes, in plain sizes", async () => {
  await renderWith();
  expect(screen.getByRole("heading", { name: "Storage" })).toBeInTheDocument();
  expect(screen.getByText("6.7 GB on this Mac")).toBeInTheDocument();
  expect(screen.getByText("38 MB")).toBeInTheDocument();
  expect(screen.getByText("61 MB")).toBeInTheDocument();
  expect(screen.getByText("6.4 GB")).toBeInTheDocument();
  expect(screen.getByRole("img", { name: /Index & summaries 38 MB, Offline email 61 MB/ })).toBeInTheDocument();
});

it("keeps all email text by default, and changes what's kept by mouse or ↑ ↓ and Space", async () => {
  const user = userEvent.setup();
  const { backend } = await renderWith();
  expect(screen.getByRole("radio", { name: /All email text/ })).toBeChecked();
  expect(screen.getByText("Default")).toBeInTheDocument();
  // J and K do nothing: Space picks the row the cursor started on.
  await user.keyboard("kK ");
  expect(backend.setOffline).toHaveBeenLastCalledWith("text");
  await user.keyboard("{ArrowUp} ");
  expect(backend.setOffline).toHaveBeenLastCalledWith("recent");
  // Can't move past either end.
  await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown} ");
  expect(backend.setOffline).toHaveBeenLastCalledWith("everything");
  // Hovering moves nothing: Space still picks the row the cursor is on.
  await user.hover(screen.getByText("Recent only"));
  await user.keyboard(" ");
  expect(backend.setOffline).toHaveBeenCalledTimes(4);
  expect(backend.setOffline).toHaveBeenLastCalledWith("everything");
  await user.click(screen.getByRole("radio", { name: /Recent only/ }));
  expect(backend.setOffline).toHaveBeenCalledTimes(5);
  expect(backend.setOffline).toHaveBeenLastCalledWith("recent");
});

it("clears the cache, rebuilds the index and sets the cache limit", async () => {
  const user = userEvent.setup();
  const { backend } = await renderWith();
  await user.click(screen.getByRole("button", { name: /^ClearC/ }));
  expect(backend.clearCache).toHaveBeenCalledOnce();
  await user.keyboard("c");
  expect(backend.clearCache).toHaveBeenCalledTimes(2);
  await user.click(screen.getByRole("button", { name: /^RebuildR/ }));
  await user.keyboard("r");
  expect(backend.rebuildIndex).toHaveBeenCalledTimes(2);
  await user.click(screen.getByRole("radio", { name: "1 GB" }));
  expect(backend.setCacheLimit).toHaveBeenCalledWith(1_000_000_000);
  // A modifier, or another key, does nothing.
  await user.keyboard("{Meta>}c{/Meta}x");
  expect(backend.clearCache).toHaveBeenCalledTimes(2);
});

it("turns reading the text inside attachments off and on", async () => {
  const user = userEvent.setup();
  const { backend } = await renderWith();
  const toggle = screen.getByRole("switch", { name: "Search inside attachments" });
  expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(screen.getByText(/so search and Ask Moss find what’s in them/)).toBeInTheDocument();
  expect(screen.queryByText(/Coming soon/)).not.toBeInTheDocument();
  await user.click(toggle);
  expect(backend.setAttachmentText).toHaveBeenCalledWith(false);
});

it("while the index rebuilds, says so and can't start another", async () => {
  const user = userEvent.setup();
  const { backend } = await renderWith({ rebuilding: true });
  expect(screen.getByText("Rebuilding. Search finds less until it’s done.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /^RebuildR/ })).toBeDisabled();
  await user.keyboard("r");
  expect(backend.rebuildIndex).not.toHaveBeenCalled();
});

it("removes the AI models, and without them offers none to remove", async () => {
  const user = userEvent.setup();
  const demo = demoAi({ state: "ready" });
  setAi(demo);
  const remove = vi.spyOn(demo, "remove");
  const { backend } = await renderWith();
  await user.click(screen.getByRole("button", { name: /^RemoveM/ }));
  expect(remove).toHaveBeenCalledOnce();
  await user.keyboard("m");
  expect(remove).toHaveBeenCalledTimes(2);
  await waitFor(() => expect(backend.storage).toHaveBeenCalledTimes(3));
});

it("without models there is nothing to remove", async () => {
  const user = userEvent.setup();
  const demo = demoAi();
  setAi(demo);
  const remove = vi.spyOn(demo, "remove");
  await renderWith({ models: 0 });
  expect(screen.getByText("Not on this Mac. Settings → AI has them.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^RemoveM/ })).not.toBeInTheDocument();
  await user.keyboard("m");
  expect(remove).not.toHaveBeenCalled();
});

it("says why something didn't work", async () => {
  const user = userEvent.setup();
  fakeMail({ clearCache: vi.fn(() => Promise.reject("The disk is full.")) });
  render(<StorageSection />);
  await screen.findByText("142 MB of 500 MB");
  await user.keyboard("c");
  expect(await screen.findByText("The disk is full.")).toBeInTheDocument();
});

it("shows nothing until the figures are in, and keys wait too", async () => {
  const user = userEvent.setup();
  const { backend } = fakeMail({ storage: vi.fn(() => new Promise<StorageInfo>(() => {})) });
  render(<StorageSection />);
  await user.keyboard("c");
  expect(screen.queryByRole("heading", { name: "Storage" })).not.toBeInTheDocument();
  expect(backend.clearCache).not.toHaveBeenCalled();
});
