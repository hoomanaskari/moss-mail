import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ai, demoAi, setAi, type AiBackend, type DemoAiStart } from "../../services/ai";
import { useAiSettings } from "../../state/aiSettings";
import { AiSection } from "./AiSection";

const status = () => useAiSettings.getState().status;
const all = { search: true, sorting: true, writing: true };
const noWriter = { search: true, sorting: true, writing: false };

async function renderAt(start?: DemoAiStart, change?: Partial<AiBackend>) {
  if (start) setAi(demoAi(start));
  if (change) setAi({ ...ai(), ...change });
  render(<AiSection />);
  await screen.findByText(/This Mac:/);
}

/** The demo AI at `start`, watching what Settings asks of it. */
function spied(start: DemoAiStart) {
  const backend = demoAi(start);
  setAi(backend);
  return {
    downloadModels: vi.spyOn(backend, "downloadModels"),
    stopModel: vi.spyOn(backend, "stopModel"),
    removeModel: vi.spyOn(backend, "removeModel"),
  };
}

/** A writing model's row. */
const row = (name: string) => screen.getByRole("radio", { name }).closest("li")!;

describe("AI settings", () => {
  it("shows this Mac, the models with the recommended one chosen, and one that needs more memory", async () => {
    await renderAt({ features: all });
    expect(screen.getByText("This Mac: M3, 16 GB memory")).toBeInTheDocument();
    expect(screen.getByText(/^Moss runs its AI on this Mac\./)).toBeInTheDocument();
    const models = screen.getByRole("radiogroup", { name: "Writing model" });
    expect(within(models).getAllByRole("radio").map((r) => r.getAttribute("aria-label"))).toEqual(["Gemma 4 E2B", "Gemma 4 E4B", "Gemma 4 26B"]);
    expect(screen.getByRole("radio", { name: "Gemma 4 E4B" })).toBeChecked();
    expect(within(row("Gemma 4 E4B")).getByText("Recommended")).toBeInTheDocument();
    expect(within(row("Gemma 4 E4B")).getByText("Ready")).toBeInTheDocument();
    expect(row("Gemma 4 E4B")).toHaveTextContent("Smarter replies, still quick.");
    expect(within(row("Gemma 4 E4B")).getByText("About 5 GB")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Gemma 4 26B" })).toBeDisabled();
    expect(within(row("Gemma 4 26B")).getByText("Needs more memory")).toBeInTheDocument();
    expect(within(row("Gemma 4 26B")).queryByRole("button")).not.toBeInTheDocument();
    // Moss's own AI only: no other AI to pick, nothing about sending email anywhere. Agents you
    // connect yourself have their own section, which names them.
    const agents = screen.getByLabelText("Your own agents");
    const elsewhere = screen.queryAllByText(/another AI|Coming soon|Claude|Ollama|OpenRouter|Who writes/).filter((el) => !agents.contains(el));
    expect(elsewhere).toEqual([]);
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("lists Apple Intelligence first on a Mac that has it: built in, ready, nothing to download or delete", async () => {
    const spy = spied({ apple: true, features: noWriter });
    render(<AiSection />);
    await screen.findByText(/This Mac:/);
    const models = screen.getByRole("radiogroup", { name: "Writing model" });
    expect(within(models).getAllByRole("radio").map((r) => r.getAttribute("aria-label"))).toEqual(["Apple Intelligence", "Gemma 4 E2B", "Gemma 4 E4B", "Gemma 4 26B"]);
    const apple = row("Apple Intelligence");
    expect(screen.getByRole("radio", { name: "Apple Intelligence" })).toBeChecked();
    expect(within(apple).getByText("Recommended")).toBeInTheDocument();
    expect(within(apple).getByText("Ready")).toBeInTheDocument();
    expect(within(apple).getByText("Built in")).toBeInTheDocument();
    expect(apple).toHaveTextContent("Built into macOS: quick, with nothing to download.");
    expect(within(apple).queryByRole("button")).not.toBeInTheDocument();
    expect(within(row("Gemma 4 E4B")).queryByText("Recommended")).not.toBeInTheDocument();
    // ⌘⌫ on it has nothing to delete.
    await userEvent.keyboard("{Control>}{Backspace}{/Control}");
    expect(spy.removeModel).not.toHaveBeenCalled();
  });

  it("goes back to Apple Intelligence from a Gemma model", async () => {
    await renderAt({ apple: true, features: all, writer: "e4b" });
    expect(screen.getByRole("radio", { name: "Gemma 4 E4B" })).toBeChecked();
    await userEvent.click(screen.getByRole("radio", { name: "Apple Intelligence" }));
    await waitFor(() => expect(status()?.writer).toBe("apple"));
    expect(screen.getByRole("radio", { name: "Apple Intelligence" })).toBeChecked();
    expect(status()?.features.writing).toBe(true);
  });

  it("says nothing about the Mac until it has looked, and its keys wait too", async () => {
    setAi({ ...ai(), status: () => new Promise(() => {}) });
    const choose = vi.spyOn(useAiSettings.getState(), "chooseModel");
    render(<AiSection />);
    expect(screen.getByRole("heading", { name: "AI" })).toBeInTheDocument();
    expect(screen.queryByText(/This Mac:/)).not.toBeInTheDocument();
    expect(screen.queryByRole("radio")).not.toBeInTheDocument();
    await userEvent.keyboard(" d{Meta>}{Backspace}{/Meta}");
    expect(choose).not.toHaveBeenCalled();
    expect(status()).toBeUndefined();
  });

  it("with nothing downloaded, each model offers its own download, and choosing one downloads it", async () => {
    const { downloadModels } = spied({});
    await renderAt();
    expect(screen.getByRole("radio", { name: "Gemma 4 E2B" })).toBeEnabled();
    expect(screen.getByRole("radio", { name: "Gemma 4 E4B" })).not.toBeChecked();
    expect(screen.getByRole("radio", { name: "Gemma 4 26B" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Download Gemma 4 E2B" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Download Gemma 4 E4B" })).toHaveTextContent("D");
    expect(screen.queryByText("Ready")).not.toBeInTheDocument();
    // The models every writer shares never show up as part of one.
    expect(screen.queryByText(/Partly downloaded|Paused at|Downloading/)).not.toBeInTheDocument();
    // Nothing is in yet, so nothing can be cancelled.
    expect(screen.queryByRole("button", { name: /^Cancel/ })).not.toBeInTheDocument();
    // Choosing a model that isn't on this Mac gets it; it writes once it's in.
    await userEvent.click(screen.getByRole("radio", { name: "Gemma 4 E2B" }));
    expect(downloadModels).toHaveBeenCalledWith(["e2b"]);
    expect(await screen.findByRole("status")).toHaveTextContent("Gemma 4 E2B is ready and now writes for you.");
  });

  it("choosing a paused or failed model goes on with it; choosing one that's downloading leaves it be", async () => {
    const { downloadModels } = spied({
      features: noWriter,
      models: { e2b: { state: "downloading", done: 1 }, e4b: { state: "missing", done: 5 } },
    });
    await renderAt();
    await userEvent.click(screen.getByRole("radio", { name: "Gemma 4 E2B" }));
    expect(downloadModels).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("radio", { name: "Gemma 4 E4B" }));
    expect(downloadModels).toHaveBeenCalledWith(["e4b"]);
  });

  it("a download running, paused or failed partway can be cancelled, which deletes what's in", async () => {
    const { removeModel } = spied({
      features: noWriter,
      models: { e2b: { state: "downloading", done: 1 }, e4b: { state: "failed", done: 4_200_000_000, message: "Can’t reach the download server." } },
    });
    await renderAt();
    expect(screen.getByRole("button", { name: "Pause Gemma 4 E2B" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel Gemma 4 E4B" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel Gemma 4 E2B" }));
    expect(removeModel).toHaveBeenCalledWith("e2b");
    // Back to the start: nothing in, so it offers its download, and no Cancel.
    expect(await screen.findByRole("button", { name: "Download Gemma 4 E2B" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel Gemma 4 E2B" })).not.toBeInTheDocument();
    cleanup();
    await renderAt({ features: noWriter, models: { e2b: { state: "missing", done: 5 }, e4b: { state: "failed", done: 0, message: "No network." } } });
    expect(screen.getByRole("button", { name: "Resume Gemma 4 E2B" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel Gemma 4 E2B" })).toBeInTheDocument();
    // A failure with nothing in has nothing to cancel.
    expect(screen.queryByRole("button", { name: "Cancel Gemma 4 E4B" })).not.toBeInTheDocument();
  });

  it("says why a download couldn't be cancelled", async () => {
    await renderAt({ features: noWriter, models: { e2b: { state: "missing", done: 5 } } }, { removeModel: () => Promise.reject({ kind: "disk", message: "can't save AI files: busy" }) });
    await userEvent.click(screen.getByRole("button", { name: "Cancel Gemma 4 E2B" }));
    expect(await screen.findByText(/busy/)).toBeInTheDocument();
  });

  it("a model partly in says so with its own numbers; the others don't", async () => {
    await renderAt({ features: noWriter, models: { e2b: { state: "missing", done: 1_200_000_000 } } });
    expect(within(row("Gemma 4 E2B")).getByText("Paused at 1.2 of 3.1 GB")).toBeInTheDocument();
    expect(within(row("Gemma 4 E2B")).getByRole("progressbar", { name: "Gemma 4 E2B download" })).toHaveAttribute("aria-valuenow", "39");
    expect(screen.getByRole("button", { name: "Resume Gemma 4 E2B" })).toBeInTheDocument();
    expect(within(row("Gemma 4 E4B")).queryByText(/Paused/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Download Gemma 4 E4B" })).toBeInTheDocument();
  });

  it("a failed download says why, keeps what is in, and a failure with nothing in says only why", async () => {
    await renderAt({ models: { e2b: { state: "failed", done: 0, message: "No network." }, e4b: { state: "failed", done: 4_200_000_000, message: "Can’t reach the download server." } } });
    expect(within(row("Gemma 4 E2B")).getByText("No network.")).toBeInTheDocument();
    expect(within(row("Gemma 4 E4B")).getByText("Can’t reach the download server. 4.2 GB is kept.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again Gemma 4 E2B" })).toBeInTheDocument();
  });

  it("a download's progress is on its own row", async () => {
    await renderAt({ features: all, models: { e2b: { state: "downloading", done: 2_100_000_000 }, "26b": { state: "queued", done: 0 } } });
    expect(within(row("Gemma 4 E2B")).getByText("Downloading… 2.1 of 3.1 GB · 67%")).toBeInTheDocument();
    expect(within(row("Gemma 4 E2B")).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "68");
    // A model downloading has no size beside it: its line says it.
    expect(within(row("Gemma 4 E2B")).queryByText("About 3.1 GB")).not.toBeInTheDocument();
    expect(within(row("Gemma 4 26B")).getByText("Waiting to download")).toBeInTheDocument();
  });

  it("Download, Pause, Resume and Try again work on that model alone", async () => {
    const { downloadModels, stopModel } = spied({
      features: noWriter,
      models: { e2b: { state: "downloading", done: 1 }, e4b: { state: "failed", done: 4_200_000_000, message: "Can’t reach the download server." } },
    });
    await renderAt();
    await userEvent.click(screen.getByRole("button", { name: "Pause Gemma 4 E2B" }));
    expect(stopModel).toHaveBeenCalledWith("e2b");
    await userEvent.click(screen.getByRole("button", { name: "Try again Gemma 4 E4B" }));
    expect(downloadModels).toHaveBeenCalledWith(["e4b"]);
    cleanup();
    const again = spied({ models: { e2b: { state: "missing", done: 5 } } });
    await renderAt();
    await userEvent.click(screen.getByRole("button", { name: "Resume Gemma 4 E2B" }));
    expect(again.downloadModels).toHaveBeenLastCalledWith(["e2b"]);
    await userEvent.click(screen.getByRole("button", { name: "Download Gemma 4 E4B" }));
    expect(again.downloadModels).toHaveBeenLastCalledWith(["e4b"]);
  });

  it("a model that finished while you chose another is ready to choose", async () => {
    await renderAt({ features: all, have: ["e2b"] });
    expect(within(row("Gemma 4 E2B")).getByText("Ready")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Gemma 4 E4B" })).toBeChecked();
    expect(screen.queryByText(/is ready and now writes for you/)).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: "Gemma 4 E2B" }));
    await waitFor(() => expect(status()?.writer).toBe("e2b"));
    expect(screen.getByRole("radio", { name: "Gemma 4 E2B" })).toBeChecked();
  });

  it("a model you download writes once it's in, and says so until your next choice", async () => {
    await renderAt({ features: all });
    await userEvent.click(screen.getByRole("button", { name: "Download Gemma 4 E2B" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Gemma 4 E2B is ready and now writes for you.");
    expect(screen.getByRole("radio", { name: "Gemma 4 E2B" })).toBeChecked();
    await userEvent.click(screen.getByRole("radio", { name: "Gemma 4 E4B" }));
    await waitFor(() => expect(screen.queryByText(/is ready and now writes for you/)).not.toBeInTheDocument());
  });

  it("each model on this Mac has a Delete, which asks once more before it deletes", async () => {
    await renderAt({ features: all, have: ["e2b"] });
    for (const name of ["d3-lite", "EmbeddingGemma", "Gemma 4 E2B", "Gemma 4 E4B"])
      expect(screen.getByRole("button", { name: `Delete ${name}` })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Delete Gemma 4 E2B" }));
    expect(status()?.downloaded).toContain("e2b");
    // Asking about another one stops asking about the first.
    await userEvent.click(screen.getByRole("button", { name: "Delete d3-lite" }));
    expect(screen.getByRole("button", { name: "Delete Gemma 4 E2B" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Delete Gemma 4 E4B" }));
    await userEvent.click(screen.getByRole("button", { name: "Yes, delete Gemma 4 E4B" }));
    await waitFor(() => expect(status()?.downloaded).toEqual(["embedder", "decider", "e2b"]));
    // Deleted means not downloaded: it offers its download again.
    expect(screen.getByRole("button", { name: "Download Gemma 4 E4B" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Gemma 4 E4B" })).not.toBeChecked();
  });

  it("says why a model couldn't be deleted", async () => {
    await renderAt({ features: all }, { removeModel: () => Promise.reject({ kind: "disk", message: "can't save AI files: busy" }) });
    await userEvent.click(screen.getByRole("button", { name: "Delete Gemma 4 E4B" }));
    await userEvent.click(screen.getByRole("button", { name: "Yes, delete Gemma 4 E4B" }));
    expect(await screen.findByText(/busy/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete Gemma 4 E4B" })).toBeInTheDocument();
  });

  it("names the sorting and search models, says when each is ready, and downloads them on their own", async () => {
    const { downloadModels } = spied({});
    await renderAt();
    const sorting = screen.getByRole("list", { name: "Sorting and search" });
    expect(sorting).toHaveTextContent("d3-liteSorts your mail and spots what needs your reply.About 1.1 GB");
    expect(sorting).toHaveTextContent("EmbeddingGemmaFinds emails by meaning, not just by exact words.About 334 MB");
    expect(within(sorting).queryByText("Ready")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Download sorting and search" }));
    expect(downloadModels).toHaveBeenCalledWith(["embedder", "decider"]);
    expect(await within(sorting).findAllByText("Ready")).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "Download sorting and search" })).not.toBeInTheDocument();
  });

  it("sorting and search say when they download, and Pause stops them", async () => {
    const { stopModel } = spied({ models: { embedder: { state: "downloading", done: 120_000_000 }, decider: { state: "queued", done: 0 } } });
    await renderAt();
    const sorting = screen.getByRole("list", { name: "Sorting and search" });
    expect(within(sorting).getByText("Downloading… 120 MB")).toBeInTheDocument();
    expect(within(sorting).getByText("Waiting to download")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Pause sorting and search" }));
    expect(stopModel.mock.calls).toEqual([["embedder"], ["decider"]]);
    cleanup();
    await renderAt({ models: { decider: { state: "failed", done: 0, message: "No network." } } });
    expect(screen.getByText("No network.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Download sorting and search" })).toBeInTheDocument();
  });

  it("a sorting model on this Mac can be deleted, after asking", async () => {
    await renderAt({ features: all });
    await userEvent.click(screen.getByRole("button", { name: "Delete d3-lite" }));
    await userEvent.click(screen.getByRole("button", { name: "Yes, delete d3-lite" }));
    await waitFor(() => expect(status()?.downloaded).not.toContain("decider"));
  });

  it("follows the AI's own status as it changes", async () => {
    await renderAt({ features: all });
    await act(() => ai().downloadModels(["e2b"]));
    expect(within(row("Gemma 4 E2B")).getByText("Ready")).toBeInTheDocument();
  });

  it("keeps the Summaries settings", async () => {
    await renderAt({ features: all });
    expect(screen.getByRole("switch", { name: /Auto summaries/ })).toBeInTheDocument();
  });
});

describe("AI settings: keys", () => {
  it("↓ ↑ move (J and K do nothing), Space chooses a model that's in, or gets one that isn't", async () => {
    const user = userEvent.setup();
    await renderAt({ features: all, have: ["e2b"] });
    await user.keyboard("jk{ArrowUp} ");
    await waitFor(() => expect(status()?.writer).toBe("e2b"));
    // Can't move past either end, and Space on a model that isn't in (or needs more memory) does nothing.
    await user.keyboard("{ArrowUp}{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown} ");
    expect(status()?.writer).toBe("e2b");
    // A modifier, or another key, does nothing.
    await user.keyboard("{Alt>}{ArrowUp}{/Alt}{Meta>}j{/Meta}x ");
    expect(status()?.writer).toBe("e2b");
    cleanup();
    const { downloadModels } = spied({ features: noWriter });
    await renderAt();
    await user.keyboard("{ArrowUp} ");
    expect(downloadModels).toHaveBeenCalledWith(["e2b"]);
  });

  it("⌘⌫ cancels the download under the cursor at once", async () => {
    const user = userEvent.setup();
    const { removeModel } = spied({ features: noWriter, models: { e4b: { state: "downloading", done: 1 } } });
    await renderAt();
    expect(screen.getByRole("button", { name: "Cancel Gemma 4 E4B" })).toHaveTextContent("Ctrl+Backspace");
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    expect(removeModel).toHaveBeenCalledWith("e4b");
  });

  it("D downloads or pauses the model under the cursor; once it's in, D does nothing", async () => {
    const user = userEvent.setup();
    const { stopModel } = spied({ features: noWriter, models: { e4b: { state: "downloading", done: 1 } } });
    await renderAt();
    expect(screen.getByRole("button", { name: "Pause Gemma 4 E4B" })).toHaveTextContent("D");
    await user.keyboard("d");
    expect(stopModel).toHaveBeenCalledWith("e4b");
    cleanup();
    const { downloadModels } = spied({ features: noWriter });
    await renderAt();
    await user.keyboard("{ArrowUp}d");
    expect(downloadModels).toHaveBeenLastCalledWith(["e2b"]);
    expect(await within(row("Gemma 4 E2B")).findByText("Ready")).toBeInTheDocument();
    await user.keyboard("d");
    expect(downloadModels).toHaveBeenCalledTimes(1);
  });

  it("⌘⌫ deletes the model under the cursor, after asking; moving or Esc stops asking", async () => {
    const user = userEvent.setup();
    await renderAt({ features: all });
    expect(screen.getByRole("button", { name: "Delete Gemma 4 E4B" })).toHaveTextContent("Ctrl+Backspace");
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    expect(screen.getByRole("button", { name: "Yes, delete Gemma 4 E4B" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("button", { name: "Delete Gemma 4 E4B" })).toBeInTheDocument();
    await user.keyboard("{Meta>}{Backspace}{/Meta}{ArrowUp}");
    expect(screen.getByRole("button", { name: "Delete Gemma 4 E4B" })).not.toHaveTextContent("Ctrl+Backspace");
    // E2B isn't on this Mac: nothing to delete.
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    expect(screen.queryByRole("button", { name: /Yes, delete/ })).not.toBeInTheDocument();
    await user.keyboard("{ArrowDown}{Meta>}{Backspace}{Backspace}{/Meta}");
    await waitFor(() => expect(status()?.features.writing).toBe(false));
    expect(screen.getByRole("button", { name: "Download Gemma 4 E4B" })).toBeInTheDocument();
  });

  it("Esc with nothing asked is left to the page", async () => {
    const user = userEvent.setup();
    const outside = vi.fn();
    await renderAt({ features: all });
    window.addEventListener("keydown", outside);
    await user.keyboard("{Escape}");
    window.removeEventListener("keydown", outside);
    expect(outside).toHaveBeenCalled();
  });
});
