import { expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetStore, work } from "../test/fixtures";
import { sendBeatMs, useStore } from "./store";
import { acknowledgeDraftHandoff, preserveCancelledDraft, stopDraftSave, retainVolatileDraft, keepUncertainDraft, currentDraftQuitBlockers, beginAttachmentImport, draftsSaved, keepDraft, localRecoveryFor, resolveLocalRecovery, settleDraftsForQuit, useDrafts, watchDrafts } from "./drafts";
import { scheduleDraft } from "./scheduled";
import type { MailBackend } from "../services/mail";

function setup(overrides: Partial<MailBackend> = {}) {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  const result = fakeMail({ saveDraft: vi.fn(async (draft) => ({ ...draft, revision: "saved" })), ...overrides });
  const stop = watchDrafts(useStore);
  useStore.getState().compose("new");useStore.getState().updateDraft({ subject: "Important draft", body: "Latest typing" });
  return { ...result, stop };
}

it("flushes typing inside the debounce before confirming quit and never sends", async () => {
  const { backend, stop } = setup();
  try {
    expect(backend.saveDraft).not.toHaveBeenCalled();
    expect(await settleDraftsForQuit()).toEqual([]);
    expect(backend.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ body: "Latest typing" }));
    expect(useDrafts.getState().unsaved).toEqual({});
    expect(backend.sendAfter).not.toHaveBeenCalled();expect(backend.scheduleSend).not.toHaveBeenCalled();
  } finally { stop(); }
});

it("waits for an in-flight save", async () => {
  let finish!: (draft: Awaited<ReturnType<MailBackend["saveDraft"]>>) => void;
  const save = vi.fn<MailBackend["saveDraft"]>(() => new Promise((resolve) => { finish = resolve; }));
  const { stop } = setup({ saveDraft: save });
  try {
    let settled = false;const result = settleDraftsForQuit().then((value) => { settled = true;return value; });
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());expect(settled).toBe(false);
    finish({ ...save.mock.calls[0][0], revision: "saved" });
    expect(await result).toEqual([]);
  } finally { stop(); }
});

it("makes only one retry and reports an unsaved draft when storage keeps refusing", async () => {
  const { backend, stop } = setup({ saveDraft: vi.fn(async () => { throw new Error("disk full"); }), draftSaveReceipt: vi.fn(async () => undefined) });
  try {
    expect(await settleDraftsForQuit()).toEqual([expect.objectContaining({ id: useStore.getState().draft!.id, subject: "Important draft" })]);
    expect(backend.saveDraft).toHaveBeenCalledTimes(2);
    expect(backend.draftSaveReceipt).toHaveBeenCalledOnce();
  } finally { stop(); }
});

it("settles a lost save acknowledgement through its receipt without writing again", async () => {
  const { backend, stop } = setup({ saveDraft: vi.fn(async () => { throw new Error("reply lost"); }), draftSaveReceipt: vi.fn(async (draft) => ({ ...draft, revision: "saved" })) });
  try {
    expect(await settleDraftsForQuit()).toEqual([]);
    expect(backend.saveDraft).toHaveBeenCalledOnce();
    expect(backend.draftSaveReceipt).toHaveBeenCalledOnce();
  } finally { stop(); }
});

it.each([false, true])("allows a conflict only after its recovery is durable, refused=%s", async (refused) => {
  const keep = vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => {
    if (refused) throw { kind: "localRecoveryRejected", message: "Recovery capacity exceeded" };
    return { status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } };
  });
  const { stop } = setup({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }), keepLocalDraftRecovery: keep });
  try {
    const result = await settleDraftsForQuit();
    expect(result).toEqual(refused ? [expect.objectContaining({ reason: "Recovery capacity exceeded" })] : []);
    expect(keep).toHaveBeenCalledTimes(refused ? 2 : 1);
  } finally { stop(); }
});

it("blocks resolved recovery chains with volatile edits", async () => {
  const { stop } = setup({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }), keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } })) });
  try {
    keepDraft(useStore.getState().draft!);await draftsSaved();
    const id = localRecoveryFor(useStore.getState().draft!)!.writer.id;
    useStore.getState().updateDraft({ body: "After review" });resolveLocalRecovery(id, "v1", null);
    expect(await settleDraftsForQuit()).toEqual([expect.objectContaining({ reason: expect.stringContaining("resolved") })]);
  } finally { stop(); }
});

it("blocks an attachment picker or import still in progress", async () => {
  const { stop } = setup();const finish = beginAttachmentImport(useStore.getState().draft!.id);
  try { expect(await settleDraftsForQuit()).toEqual([expect.objectContaining({ reason: expect.stringContaining("attachment") })]); }
  finally { finish();stop(); }
});

it("keeps an unconfirmed receipt read failure visible, including untitled drafts", async () => {
  const { stop } = setup({ saveDraft: vi.fn(async () => { throw new Error("reply lost"); }), draftSaveReceipt: vi.fn(async () => { throw new Error("receipt unreadable"); }) });
  try {
    useStore.getState().updateDraft({ subject: "" });
    expect(await settleDraftsForQuit()).toEqual([expect.objectContaining({ subject: "No subject", reason: "The last save has not been confirmed." })]);
  } finally { stop(); }
});

it("blocks new typing that arrives while the retry is waiting", async () => {
  let finish!: (draft: Awaited<ReturnType<MailBackend["saveDraft"]>>) => void;
  const save = vi.fn<MailBackend["saveDraft"]>().mockRejectedValueOnce(new Error("offline")).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const { stop } = setup({ saveDraft: save, draftSaveReceipt: vi.fn(async () => undefined) });
  try {
    const settling = settleDraftsForQuit();await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(2));
    useStore.getState().updateDraft({ body: "Typed during the retry" });
    finish({ ...save.mock.calls[1][0], revision: "saved" });
    expect(await settling).toEqual([expect.objectContaining({ reason: "New edits are still waiting to be saved." })]);
  } finally { stop(); }
});

it("blocks imports whose draft row has not appeared yet", async () => {
  const finish = beginAttachmentImport("not-yet-in-list");
  try { expect(await settleDraftsForQuit()).toEqual([{ id: "not-yet-in-list", subject: "No subject", reason: "An attachment is still being selected or imported." }]); }
  finally { finish(); }
});

it("does not forget an unpersisted conflict when another editing chain replaces the list row", async () => {
  const save = vi.fn<MailBackend["saveDraft"]>().mockRejectedValueOnce({ kind: "draftConflict" }).mockImplementation(async (draft) => ({ ...draft, revision: "winner" }));
  const { stop } = setup({ saveDraft: save, keepLocalDraftRecovery: vi.fn(async () => { throw new Error("Recovery disk full"); }) });
  keepDraft(useStore.getState().draft!);await draftsSaved();
  const mine = useStore.getState().draft!;stop();
  keepDraft({ ...mine, body: "Other editor", revision: "winner-before" });await draftsSaved();
  expect(useDrafts.getState().unsaved).toEqual({});
  expect(await settleDraftsForQuit()).toEqual([expect.objectContaining({ reason: "Recovery disk full" })]);
  expect(useDrafts.getState().drafts[0].body).toBe("Other editor");
});

it("reports a failed deletion without silently deleting a newer row on retry", async () => {
  const { backend, stop } = setup({ deleteDraft: vi.fn(async () => { throw new Error("delete reply lost"); }) });
  try {
    keepDraft(useStore.getState().draft!);await draftsSaved();
    useStore.getState().discardDraft();await draftsSaved();
    expect(useDrafts.getState().unsaved).toEqual({});
    expect(await settleDraftsForQuit()).toEqual([expect.objectContaining({reason:"A draft deletion has not been confirmed. Review the draft before trying again."})]);
    expect(backend.deleteDraft).toHaveBeenCalledOnce();
  } finally {stop();}
});

it.each(["newer", "other-chain", "untracked"])("preserves unsaved content outside a native handoff: %s",async(kind)=>{
 const {stop}=setup({saveDraft:vi.fn(async()=>{throw new Error("disk full");})});
 try {
  const source=useStore.getState().draft!;keepDraft(source);await draftsSaved();
  if(kind==="newer") useStore.getState().updateDraft({body:"Newer than send"});
  if(kind==="other-chain") {stop();keepDraft({...source,body:"Other chain"});await draftsSaved();}
  const before=useDrafts.getState().unsaved[source.id];
  acknowledgeDraftHandoff(kind==="untracked"?{...source}:source);
  expect(useDrafts.getState().unsaved[source.id]).toBe(before);
  expect(useDrafts.getState().retired[source.id]).toBeUndefined();
 } finally {stop();}
});

it("reports a journal resolved by another client while the latest text is still local",async()=>{
 const {stop}=setup({saveDraft:vi.fn(async()=>{throw {kind:"draftConflict"};}),keepLocalDraftRecovery:vi.fn(async()=>({status:"resolved" as const,adoptedId:null}))});
 try {expect(await settleDraftsForQuit()).toEqual([expect.objectContaining({reason:"This recovery was resolved. Keep the remaining local edits separately or discard them."})]);}
 finally {stop();}
});

it("saves a refused send after the composer closed instead of retaining text only in Undo",async()=>{
 vi.useFakeTimers();let reject!:(error:Error)=>void;
 const {backend,stop}=setup({sendAfter:vi.fn(()=>new Promise<boolean>((_resolve,fail)=>{reject=fail;}))});
 try {
  useStore.getState().send();
  expect(currentDraftQuitBlockers()).toEqual([expect.objectContaining({reason:"This draft is still being handed off or restored."})]);
  await vi.advanceTimersByTimeAsync(sendBeatMs);
  expect(useStore.getState().draft).toBeUndefined();reject(new Error("Preparation refused"));
  await vi.advanceTimersByTimeAsync(0);await draftsSaved();
  expect(backend.saveDraft).toHaveBeenCalledWith(expect.objectContaining({body:"Latest typing"}));
  expect(useDrafts.getState().drafts.some(draft=>draft.body==="Latest typing")).toBe(true);
  expect(await settleDraftsForQuit()).toEqual([]);
 } finally {stop();vi.useRealTimers();}
});
it("retains the local snapshot when both scheduling and cancellation replies fail",async()=>{
 const {backend,stop}=setup({scheduleSend:vi.fn(async()=>{throw new Error("Schedule lost");}),unscheduleSend:vi.fn(async()=>{throw new Error("Cancel lost");})});
 try {
  scheduleDraft(useStore.getState().draft!,Date.now()+60000);
  await vi.waitFor(()=>expect(useStore.getState().toast?.text).toContain("Couldn’t confirm scheduling"));
  await draftsSaved();expect(backend.saveDraft).not.toHaveBeenCalled();
  expect(backend.keepLocalDraftRecovery).toHaveBeenCalledWith(expect.any(String),undefined,expect.objectContaining({body:"Latest typing"}));
  expect(useDrafts.getState().drafts.some(draft=>draft.body==="Latest typing")).toBe(true);
 } finally {stop();}
});

it("releases only the native-acknowledged volatile snapshot",()=>{
 resetStore([],[work]);useStore.getState().compose("new");useStore.getState().updateDraft({body:"Sent snapshot"});
 const sent=useStore.getState().draft!;
 const first=retainVolatileDraft(sent);const other=retainVolatileDraft({...sent,id:"other",body:"Another draft"});
 acknowledgeDraftHandoff(sent);
 expect(first.pending()).toBe(false);expect(other.pending()).toBe(true);
 expect(currentDraftQuitBlockers()).toEqual([expect.objectContaining({id:"other"})]);
 first.release();other.release();expect(useDrafts.getState().volatile.size).toBe(0);
});
it("freezes an uncertain handoff into one journal and reports it while storage is pending",async()=>{
 let finish!:(result:Awaited<ReturnType<MailBackend["keepLocalDraftRecovery"]>>)=>void;
 const keep=vi.fn<MailBackend["keepLocalDraftRecovery"]>(()=>new Promise(resolve=>{finish=resolve;}));
 const {backend,stop}=setup({keepLocalDraftRecovery:keep});
 try {
  const draft=useStore.getState().draft!;stopDraftSave(draft.id);keepUncertainDraft(draft);
  await vi.waitFor(()=>expect(keep).toHaveBeenCalledOnce());
  const id=localRecoveryFor(draft)!.writer.id;keepUncertainDraft(draft);
  expect(localRecoveryFor(draft)!.writer.id).toBe(id);
  expect(currentDraftQuitBlockers()).toEqual([expect.objectContaining({reason:"These local recovery edits have not been confirmed saved."})]);
  finish({status:"saved",recovery:{id,version:"v1",draft:keep.mock.calls[0][2],missingFiles:[],updatedAt:1}});
  await draftsSaved();expect(backend.saveDraft).not.toHaveBeenCalled();expect(keep).toHaveBeenCalledOnce();
 } finally {stop();}
});

it("persists an early-cancelled send even when Undo's committed reply is lost",async()=>{
 vi.useFakeTimers();let finish!:(sent:boolean)=>void;
 const {backend,stop}=setup({sendAfter:vi.fn(()=>new Promise<boolean>(resolve=>{finish=resolve;})),cancelSend:vi.fn(async()=>{throw new Error("Reply lost after cancellation");}),keepLocalDraftRecovery:vi.fn<MailBackend["keepLocalDraftRecovery"]>(async(id,_previous,draft)=>({status:"saved",recovery:{id,version:"v1",draft,missingFiles:[],updatedAt:1}}))});
 try {
  useStore.getState().send();await vi.advanceTimersByTimeAsync(sendBeatMs);
  useStore.getState().undo();await vi.advanceTimersByTimeAsync(0);
  finish(false);await vi.advanceTimersByTimeAsync(0);await draftsSaved();
  expect(backend.keepLocalDraftRecovery).toHaveBeenCalledWith(expect.any(String),undefined,expect.objectContaining({body:"Latest typing"}));
  expect(useDrafts.getState().drafts[0].body).toBe("Latest typing");
  expect(await settleDraftsForQuit()).toEqual([]);
 } finally {stop();vi.useRealTimers();}
});
it("a current recovery supersedes an unreadable old save receipt at quit",async()=>{
 const {stop}=setup({saveDraft:vi.fn(async()=>{throw new Error("Lost save reply");}),draftSaveReceipt:vi.fn(async()=>{throw new Error("Receipt unavailable");}),keepLocalDraftRecovery:vi.fn<MailBackend["keepLocalDraftRecovery"]>(async(id,_previous,draft)=>({status:"saved",recovery:{id,version:"v1",draft,missingFiles:[],updatedAt:1}}))});
 try {
  const draft=useStore.getState().draft!;stopDraftSave(draft.id);keepDraft(draft);await draftsSaved();
  expect(useDrafts.getState().unconfirmed.size).toBe(1);keepUncertainDraft(draft);await draftsSaved();
  expect(await settleDraftsForQuit()).toEqual([]);expect(useDrafts.getState().unconfirmed.size).toBe(0);
 } finally {stop();}
});
it("a prepared native snapshot releases its own volatile hold without touching a foreign row",()=>{
 resetStore([],[work]);useStore.getState().compose("new");useStore.getState().updateDraft({body:"Prepared"});
 const sent=useStore.getState().draft!;const retained=retainVolatileDraft(sent);
 const other={...sent,body:"Other editor",updatedAt:1};useDrafts.setState({drafts:[other],unsaved:{[other.id]:other}});
 acknowledgeDraftHandoff(sent);
 expect(retained.pending()).toBe(false);expect(useDrafts.getState().drafts).toEqual([other]);expect(useDrafts.getState().unsaved[other.id]).toBe(other);
 retained.release();
});
it("does not hide a native draft when Undo confirms cancellation without a restoration payload",async()=>{
 vi.useFakeTimers();let finish!:(sent:boolean)=>void;
 const {backend,stop}=setup({sendAfter:vi.fn(()=>new Promise<boolean>(resolve=>{finish=resolve;})),cancelSend:vi.fn(async()=>({cancelled:true,early:false})),drafts:vi.fn(async()=>[])});
 try {
  const id=useStore.getState().draft!.id;useStore.getState().send();await vi.advanceTimersByTimeAsync(sendBeatMs);
  useStore.getState().undo();await vi.advanceTimersByTimeAsync(0);await draftsSaved();
  expect(useDrafts.getState().retired[id]).toBeUndefined();expect(backend.drafts).toHaveBeenCalled();
  finish(false);await vi.advanceTimersByTimeAsync(0);
 } finally {stop();vi.useRealTimers();}
});

it("keeps a cancelled older chain separately without replacing another editor's row",async()=>{
 const keep=vi.fn<MailBackend["keepLocalDraftRecovery"]>(async(id,_previous,draft)=>({status:"saved",recovery:{id,version:"v1",draft,missingFiles:[],updatedAt:1}}));
 const {backend,stop}=setup({keepLocalDraftRecovery:keep});const original=useStore.getState().draft!;stop();
 keepDraft({...original,body:"Other editor",revision:"winner"});await draftsSaved();
 preserveCancelledDraft(original);await draftsSaved();keepUncertainDraft(original);await draftsSaved();
 expect(useDrafts.getState().drafts[0].body).toBe("Other editor");
 expect(useDrafts.getState().status[original.id]).toBe("saved");expect(backend.saveDraft).toHaveBeenCalledOnce();
 expect(keep).toHaveBeenCalledOnce();expect(localRecoveryFor(original)?.latest?.body).toBe("Latest typing");
 expect(await settleDraftsForQuit()).toEqual([]);
});
it("saves a cancellation result that arrives without an Undo acknowledgement",async()=>{
 vi.useFakeTimers();let finish!:(sent:boolean)=>void;
 const {backend,stop}=setup({sendAfter:vi.fn(()=>new Promise<boolean>(resolve=>{finish=resolve;}))});
 try {
  useStore.getState().send();await vi.advanceTimersByTimeAsync(sendBeatMs);
  finish(false);await vi.advanceTimersByTimeAsync(0);await draftsSaved();
  expect(backend.saveDraft).toHaveBeenCalledWith(expect.objectContaining({body:"Latest typing"}));
  expect(await settleDraftsForQuit()).toEqual([]);
 } finally {stop();vi.useRealTimers();}
});

it.each([false,true])("native handoff clears only save receipts covered by its snapshot, newer=%s",async(newer)=>{
 const {stop}=setup({saveDraft:vi.fn(async()=>{throw new Error("Reply lost");})});
 try {
  const original=useStore.getState().draft!;const retained=retainVolatileDraft(original);
  if(newer) useStore.getState().updateDraft({body:"Newer edit"});
  keepDraft(useStore.getState().draft!);await draftsSaved();expect(useDrafts.getState().unconfirmed.size).toBe(1);
  acknowledgeDraftHandoff(original);expect(useDrafts.getState().unconfirmed.size).toBe(newer?1:0);retained.release();
 } finally {stop();}
});

it("replaying an older uncertain snapshot cannot replace newer journal text behind a foreign row",async()=>{
 const keep=vi.fn<MailBackend["keepLocalDraftRecovery"]>(async(id,_previous,draft)=>({status:"saved",recovery:{id,version:draft.body,draft,missingFiles:[],updatedAt:1}}));
 const {stop}=setup({keepLocalDraftRecovery:keep});const old=useStore.getState().draft!;
 useStore.getState().updateDraft({body:"Newest local recovery"});const newest=useStore.getState().draft!;
 keepUncertainDraft(newest);await draftsSaved();stop();
 keepDraft({...old,body:"Other editor"});await draftsSaved();
 keepUncertainDraft(old);await draftsSaved();
 expect(localRecoveryFor(old)?.latest?.body).toBe("Newest local recovery");
 expect(keep).toHaveBeenCalledOnce();expect(useDrafts.getState().drafts[0].body).toBe("Other editor");
});
