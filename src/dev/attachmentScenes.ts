/** Dev scenes for attachments: thumbnails where a file can have one, its kind's icon where not. */
import brochure from "../../design-assets/attachments/brochure.jpg";
import photo from "../../design-assets/attachments/photo.jpg";
import { people } from "../data/demo";
import type { Attachment, DraftFile } from "../data/types";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

/** The files in the design frame "Attachments": a photo, a PDF, a document, a zip and something unknown. */
const mixed: Attachment[] = [
  { name: "Studio roof.jpg", size: "1.9 MB", preview: photo },
  { name: "WelcomeBrochure.pdf", size: "2.8 MB", preview: brochure },
  { name: "Onboarding plan.docx", size: "84 KB" },
  { name: "Brand assets.zip", size: "12.4 MB" },
  { name: "export.dat", size: "3 KB" },
];

/** Files you attached to a new email: two with thumbnails, two with their kind's icon. */
const attached: DraftFile[] = [
  { id: "s1", name: "Studio roof.jpg", bytes: 1_992_294, contentType: "image/jpeg" },
  { id: "s2", name: "WelcomeBrochure.pdf", bytes: 2_936_013, contentType: "application/pdf" },
  { id: "s3", name: "Onboarding plan.docx", bytes: 86_016, contentType: "" },
  { id: "s4", name: "Brand assets.zip", bytes: 13_002_342, contentType: "application/zip" },
];

/** A new email to Priya with `files` attached, their thumbnails already drawn. */
function composeWith(files: DraftFile[]) {
  const s = useStore.getState();
  s.compose("new");
  s.updateDraft({
    to: [people.priya],
    subject: "Studio photos and the brochure",
    body: "Hi Priya,\n\nI’ve attached the roof photo, the brochure and the onboarding plan. The brand assets are in the zip.\n\nMaya",
    signatureId: null,
    attachments: files,
  });
  useStore.setState({ previews: { "draft:s1": photo, "draft:s2": brochure } });
}

export const attachmentScenes: Record<string, Scene> = {
  "composer-attachments": {
    shows: /Brand assets\.zip/,
    apply: () => composeWith(attached),
  },
  "composer-attachments-too-big": {
    shows: /Outlook sends up to 20 MB/,
    apply: () => composeWith([...attached, { id: "s5", name: "Shoot film.mov", bytes: 41_943_040, contentType: "video/quicktime" }]),
  },
  "conversation-attachments": {
    shows: /Brand assets\.zip/,
    apply: () => {
      useStore.setState((s) => ({
        threads: s.threads.map((t) =>
          t.id !== "q4-budget" ? t : { ...t, messages: t.messages.map((m) => ({ ...m, attachments: mixed })) },
        ),
      }));
      useStore.getState().open("q4-budget");
    },
  },
};
