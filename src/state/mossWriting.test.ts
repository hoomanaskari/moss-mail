import { beforeEach, expect, it, vi } from "vitest";
import { demoReplies, setReplies, type ReplyBackend } from "../services/replies";
import { resetStore, thread } from "../test/fixtures";
import { useAiSettings } from "./aiSettings";
import { useMossWriting } from "./mossWriting";
import { useSignatures } from "./signatures";
import { useStore } from "./store";

const m = () => useMossWriting.getState();
const s = () => useStore.getState();
const shorter = { job: { kind: "shorter" as const }, lead: "Moss made it shorter" };
const note = { job: { kind: "instructions" as const, instructions: "say yes" }, lead: "Moss wrote this from your note", note: "say yes" };

beforeEach(() => {
  resetStore([thread("a")]);
  m().reset();
  setReplies(demoReplies());
  s().open("a");
  s().compose("reply");
  s().updateDraft({ body: "Hi Dana,\n\nYes.", bodyHtml: "<p>Hi Dana,</p><p>Yes.</p>" });
});

it("⌘J opens the menu and Esc closes it", () => {
  m().openMenu();
  expect(m().menuOpen).toBe(true);
  m().closeMenu();
  expect(m().menuOpen).toBe(false);
});

it("writes into the composer, streaming, and says what it did", async () => {
  const seen: string[] = [];
  const stop = useMossWriting.subscribe((st) => st.status === "writing" && seen.push(st.streamed));
  const version = m().version;
  m().openMenu();
  const writing = m().write(shorter);
  expect(m()).toMatchObject({ menuOpen: false, status: "writing", before: "Hi Dana,\n\nYes." });
  // Your words stay as they are until it's done.
  expect(s().draft?.body).toBe("Hi Dana,\n\nYes.");
  await writing;
  stop();
  expect(seen.at(-1)).toBe("Hi Dana,");
  expect(s().draft).toMatchObject({ body: "Hi Dana,", bodyHtml: "<p>Hi Dana,</p>" });
  expect(m()).toMatchObject({ status: "wrote", pick: shorter, version: version + 1 });
});

it("sends the draft's conversation and account, and your note", async () => {
  const write = vi.fn(async () => "Yes!");
  setReplies({ ...demoReplies(), write });
  await m().write(note);
  expect(write).toHaveBeenCalledWith(
    expect.objectContaining({ threadId: "a", accountId: "work", job: note.job, text: "Hi Dana,\n\nYes." }),
    expect.any(Function),
  );
  expect(s().draft?.body).toBe("Yes!");
});

it("sends the signature on the draft, so the reply ends to fit it", async () => {
  const write = vi.fn(async () => "Yes!");
  setReplies({ ...demoReplies(), write });
  useSignatures.setState({ signatures: [{ id: "short", name: "Short", lines: ["Best", "Maya"] }] });
  s().updateDraft({ signatureId: "short" });
  await m().write(note);
  expect(write).toHaveBeenLastCalledWith(expect.objectContaining({ signature: ["Best", "Maya"] }), expect.any(Function));
  s().updateDraft({ signatureId: null });
  await m().write(note);
  expect(write).toHaveBeenLastCalledWith(expect.objectContaining({ signature: [] }), expect.any(Function));
});

it("Undo puts your words back", async () => {
  await m().write(shorter);
  const version = m().version;
  m().undo();
  expect(s().draft?.body).toBe("Hi Dana,\n\nYes.");
  expect(m()).toMatchObject({ status: undefined, version: version + 1 });
  // Nothing left to undo.
  m().undo();
  expect(m().version).toBe(version + 1);
});

it("typing in the text drops the strip, but keeps a failure on screen", async () => {
  await m().write(shorter);
  m().dismiss();
  expect(m().status).toBeUndefined();
  setReplies(demoReplies({ state: "failed" }));
  await m().write(shorter);
  m().dismiss();
  expect(m().status).toBe("failed");
});

it("Stop keeps what it wrote so far", async () => {
  setReplies(demoReplies({ state: "writing" }));
  const writing = m().write({ job: { kind: "accept" }, lead: "Moss wrote a yes" });
  await Promise.resolve();
  m().stop();
  await writing;
  expect(m().status).toBe("wrote");
  expect(s().draft?.body).toBe("Hi,\n\nYes, happy to.");
  // Not writing: nothing to stop.
  m().stop();
});

it("without the model, offers the download and settings", async () => {
  const download = vi.fn(async () => {});
  useAiSettings.setState({ download });
  setReplies(demoReplies({ state: "missing" }));
  await m().write(shorter);
  expect(m().status).toBe("missing");
  expect(s().draft?.body).toBe("Hi Dana,\n\nYes.");
  m().download();
  expect(download).toHaveBeenCalledOnce();
  expect(m().status).toBeUndefined();
  m().settings();
  expect(s()).toMatchObject({ view: "settings", settingsSection: "ai" });
});

it("a failure leaves your draft; Try again asks again", async () => {
  setReplies(demoReplies({ state: "failed" }));
  await m().write(shorter);
  expect(m().status).toBe("failed");
  setReplies(demoReplies());
  await m().retry();
  expect(s().draft?.body).toBe("Hi Dana,");
  m().reset();
  await m().retry();
  expect(m().status).toBeUndefined();
});

it("the menu opened on another draft starts clean", async () => {
  await m().write(shorter);
  s().compose("new");
  m().openMenu();
  expect(m()).toMatchObject({ draftId: s().draft?.id, menuOpen: true, status: undefined });
});

it("Undo after a failure just puts the strip away; while writing it waits", async () => {
  setReplies(demoReplies({ state: "failed" }));
  await m().write(shorter);
  m().undo();
  expect(m().status).toBeUndefined();
  expect(s().draft?.body).toBe("Hi Dana,\n\nYes.");
  setReplies(demoReplies({ state: "writing" }));
  const writing = m().write(shorter);
  m().undo();
  expect(m().status).toBe("writing");
  m().stop();
  await writing;
});

it("Undo leaves another draft alone", async () => {
  await m().write(shorter);
  s().compose("new");
  s().updateDraft({ body: "New one" });
  m().undo();
  expect(s().draft?.body).toBe("New one");
});

it("writing that ends once another draft is open is dropped", async () => {
  let finish: (t: string) => void = () => {};
  setReplies({ ...demoReplies(), write: () => new Promise((resolve) => (finish = resolve)) });
  const writing = m().write(shorter);
  s().compose("new");
  finish("Late");
  await writing;
  expect(s().draft?.body).toBe("");
});

it("writing that ends after you left or undid is dropped", async () => {
  let finish: (t: string) => void = () => {};
  let fail: (e: unknown) => void = () => {};
  const slow: ReplyBackend = {
    ...demoReplies(),
    write: (_r, onText) => {
      onText("Late");
      return new Promise((resolve, reject) => ((finish = resolve), (fail = reject)));
    },
  };
  setReplies(slow);
  const first = m().write(shorter);
  m().reset();
  finish("Late text");
  await first;
  expect(s().draft?.body).toBe("Hi Dana,\n\nYes.");
  expect(m().status).toBeUndefined();
  const second = m().write(shorter);
  m().reset();
  fail({ kind: "model" });
  await second;
  expect(m().status).toBeUndefined();
});

it("with no draft open, does nothing", async () => {
  useStore.setState({ draft: undefined });
  await m().write(shorter);
  expect(m().status).toBeUndefined();
});
