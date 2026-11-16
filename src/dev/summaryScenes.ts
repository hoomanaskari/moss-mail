/**
 * The summary states drawn in design.pen (section 1 of docs/ai-features-design.md on the design
 * branch), on the demo's Q4 budget email from Daniel.
 */
import { demoAi, setAi } from "../services/ai";
import { demoSummaries, demoView, setSummaries, type DemoSummaryStart, type SummaryRequest, type SummaryRule } from "../services/summaries";
import { useStore } from "../state/store";
import { useSummary } from "../state/summary";
import type { Scene } from "./scenes";

const custom: SummaryRequest = { kind: "custom", instruction: "As a short list: amount, deadline, what’s new" };
const rule = (sender: string, request: SummaryRequest, extra: Partial<SummaryRule> = {}): SummaryRule => ({ id: 1, sender, ...request, createdAt: Date.UTC(2026, 9, 20), ...extra });

/** The demo summaries started at `start`, then the Q4 budget email opened. */
const open = (start: DemoSummaryStart) => {
  setSummaries(demoSummaries(start));
  useStore.getState().open("q4-budget");
};

/** As `open`, then once the email has loaded its summaries, `then` (asking for one, opening the menu). */
const openThen = (start: DemoSummaryStart, then: () => void) => () => {
  open(start);
  setTimeout(then, 50);
};

const ask = (request: SummaryRequest) => () => void useSummary.getState().summarize(request);

export const summaryScenes: Record<string, Scene> = {
  "summary-menu": { shows: /Summarise as/, apply: openThen({}, () => useSummary.getState().setMenu(true)) },
  "summary-short": { shows: /Always summarise mail from daniel@northwind\.co like this/, apply: openThen({}, ask({ kind: "short" })) },
  "summary-actions-rule": {
    shows: /Always for Daniel Okafor/,
    apply: () =>
      open({
        rules: [rule("daniel@northwind.co", { kind: "actions" }, { name: "Daniel Okafor" })],
        kept: { "q4-budget": demoView({ kind: "actions" }, { why: "rule" }) },
      }),
  },
  "summary-facts-domain": {
    shows: /Always for everyone at northwind\.co/,
    apply: () => open({ rules: [rule("northwind.co", { kind: "facts" })], kept: { "q4-budget": demoView({ kind: "facts" }, { why: "rule" }) } }),
  },
  // Key facts of a newsletter: links by name, one per line, the first five and the rest on asking.
  "summary-facts-links": {
    shows: /3 more/,
    apply: () =>
      open({
        kept: {
          "q4-budget": demoView(
            { kind: "facts" },
            {
              content: {
                format: "facts",
                items: [
                  { kind: "date", value: "Thu 12 Nov", detail: "Team plan private beta closes" },
                  { kind: "link", value: "https://keyward.example/changelog/2026-10-14-shared-workspaces", detail: "Shared workspaces" },
                  { kind: "link", value: "https://keyward.example/changelog/2026-10-21-faster-exports", detail: "Faster exports" },
                  { kind: "link", value: "https://keyward.example/changelog/2026-10-28-new-limits-for-trial-workspaces-and-shared-test-folders" },
                  { kind: "link", value: "https://keyward.example/changelog/2026-10-26-dark-mode", detail: "Dark mode" },
                  { kind: "link", value: "https://video.example/watch?v=teamplan", detail: "Team plan demo" },
                  { kind: "link", value: "https://events.example/keyward-office-hours", detail: "Office hours sign-up" },
                  { kind: "link", value: "https://keyward.example/blog/2026-10-25-styling-the-share-menu", detail: "Styling the share menu" },
                  { kind: "link", value: "https://feedback.keyward.example/", detail: "Feedback board" },
                ],
              },
            },
          ),
        },
      }),
  },
  "summary-detailed": { shows: /Detailed summary/, apply: () => open({ kept: { "q4-budget": demoView({ kind: "detailed" }) } }) },
  // A model that writes Markdown anyway: headings, numbered points and emphasis show as they mean.
  "summary-markdown": {
    shows: /The numbers are final/,
    apply: () =>
      open({
        kept: {
          "q4-budget": demoView(
            { kind: "detailed" },
            { content: { format: "text", text: "**The numbers are final.** Daniel moved *$12k* from travel to the offsite.\n\n**What matters:**\n1. Sign off by **Wednesday**\n2. The `Q4-budget.xlsx` sheet has the new totals\n\n## Also\n- Lena joins the review" } },
          ),
        },
      }),
  },
  "summary-custom": { shows: /like this|this way/, apply: openThen({}, ask(custom)) },
  "summary-auto": { shows: /Because it’s in Important/, apply: () => open({ kept: { "q4-budget": demoView({ kind: "short" }, { why: "important" }) } }) },
  "summary-loading": { shows: /Summarising/, apply: openThen({ state: "loading" }, ask({ kind: "detailed" })) },
  // Part-way written: the summary so far shows as it streams in.
  "summary-writing": { shows: /Daniel updated the Q4 design budget/, apply: openThen({ state: "writing" }, ask({ kind: "detailed" })) },
  "summary-no-model": {
    shows: /Summaries need the writing model/,
    apply: openThen({ state: "notReady" }, () => {
      setAi(demoAi());
      void useSummary.getState().summarize({ kind: "short" });
    }),
  },
  "summary-failed": { shows: /Couldn’t summarise this one/, apply: openThen({ state: "failed" }, ask({ kind: "facts" })) },
  "settings-ai-summaries": {
    shows: /Auto summaries/,
    apply: () => {
      setAi(demoAi({ state: "ready" }));
      setSummaries(demoSummaries());
      useStore.getState().openSettings("ai");
    },
  },
};
