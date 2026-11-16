import { expect, it } from "vitest";
import { mail } from "../services/mail";
import { useQuit } from "../state/quitState";
import { useStore } from "../state/store";
import { recoveryScenes } from "./recoveryScenes";

it("provides a synthetic reviewable recovery without pretending it is a desktop journal", async () => {
  recoveryScenes["local-draft-recovery"].apply();
  const entries = await mail().localDraftRecoveryIndex();
  expect(entries).toHaveLength(1);
  const copy = await mail().localDraftRecovery(entries[0].id);
  expect(copy?.draft.to[0].email).toBe("alex@example.test");
  expect(copy?.missingFiles).toEqual(["missing"]);
  expect(useStore.getState().view).toBe("drafts");
  await expect(mail().adoptLocalDraftRecovery(entries[0].id,entries[0].version,"new","work",["missing"])).rejects.toThrow("desktop app");
});
it("shows an unsaved-work quit review with synthetic text", () => {
  recoveryScenes["quit-unsaved-drafts"].apply();
  expect(useQuit.getState()).toMatchObject({ checking:false,blockers:[{subject:"Cabinet plans"}] });
});
