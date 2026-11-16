import { beforeEach, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetStore } from "../test/fixtures";
import { resetInterruptedSends, sayInterruptedSends } from "./interruptedSends";
import { useStore } from "./store";

const s = () => useStore.getState();
const stopped = (id: string, subject: string) => ({ id, subject, at: 1 });
beforeEach(() => { resetStore([]); resetInterruptedSends(); });

it("says once that an email you sent didn't go and is back in Drafts, with a way there", async () => {
  const { backend } = fakeMail({ interruptedSends: vi.fn(async () => [stopped("op", "Contract")]) });
  await sayInterruptedSends();
  expect(s().toast).toMatchObject({ text: "Moss stopped before sending “Contract”. It is back in Drafts.", stays: true, undo: false, show: { label: "Show drafts" } });
  expect(backend.dismissInterruptedSends).toHaveBeenCalledWith(["op"]);
  s().toast!.show!.run();
  expect(s().view).toBe("drafts");
  // The engine may say so again before it hears the note was shown: not twice.
  useStore.setState({ toast: undefined });
  await sayInterruptedSends();
  expect(s().toast).toBeUndefined();
  expect(backend.dismissInterruptedSends).toHaveBeenCalledOnce();
});

it("counts several, and names an email without a subject plainly", async () => {
  fakeMail({ interruptedSends: vi.fn().mockResolvedValueOnce([stopped("a", "One"), stopped("b", "Two")]).mockResolvedValueOnce([stopped("c", "  ")]) });
  await sayInterruptedSends();
  expect(s().toast?.text).toBe("Moss stopped before sending 2 emails. They are back in Drafts.");
  await sayInterruptedSends();
  expect(s().toast?.text).toBe("Moss stopped before sending “No subject”. It is back in Drafts.");
});

it("says nothing when every email went", async () => {
  const { backend } = fakeMail();
  await sayInterruptedSends();
  expect(s().toast).toBeUndefined();
  expect(backend.dismissInterruptedSends).not.toHaveBeenCalled();
});
