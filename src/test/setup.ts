import "@testing-library/jest-dom/vitest";
import { TestResizeObserver } from "./resize";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { demoAi, setAi } from "../services/ai";
import { demoAsk, setAsk } from "../services/ask";
import { demoMail, setMail } from "../services/mail";
import { useSync } from "../services/live";
import { forgetPreloaded } from "../services/preload";
import { useAiSettings } from "../state/aiSettings";
import { resetAskPanel } from "../state/askPanel";
import { useDrafts } from "../state/drafts";
import { useQuit } from "../state/quitState";
import { useStorageSettings } from "../state/storageSettings";

afterEach(() => {
  cleanup();
  useQuit.setState({ token: undefined, exiting: false, checking: false, blockers: [], error: undefined });
  setMail(demoMail);
  forgetPreloaded();
  setAi(demoAi());
  setAsk(demoAsk());
  resetAskPanel();
  useAiSettings.setState({ status: undefined, problem: undefined });
  useStorageSettings.setState({ info: undefined, problem: undefined });
  useSync.setState({ progress: {}, problems: {} });
  useDrafts.setState({ drafts: [], status: {}, acknowledgements: {}, unsaved: {}, unconfirmed: new Map(), recoveries: new Map(), volatile: new Map(), retired: {}, cursorId: undefined });
});

// jsdom has no layout engine. Stand-ins for the browser APIs the UI and the editor call.
globalThis.ResizeObserver ??= TestResizeObserver;
const emptyRects = () => ({ length: 0, item: () => null, [Symbol.iterator]: [][Symbol.iterator] }) as unknown as DOMRectList;
Range.prototype.getClientRects ??= emptyRects;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();
Element.prototype.scrollIntoView ??= () => {};
Element.prototype.scrollTo ??= () => {};
Element.prototype.scrollBy ??= () => {};
document.elementFromPoint ??= () => null;

// jsdom has no top layer; native focus trapping needs browser verification.
HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute("open", ""); };
