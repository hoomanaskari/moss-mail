/**
 * Dev scenes for Ask Moss, as drawn in design.pen (frames xMcPb, LC7J6, dqC1r, a9w3A, oXgeP and
 * AkCIm). The panel's turns are set as they stand once each answer lands, so every scene is still.
 */
import type { Message, Person, Thread } from "../data/types";
import { people } from "../data/demo";
import { startTurn, type ChatTurn } from "../services/ask";
import { useAskPanel, type AskPanelState } from "../state/askPanel";
import type { SavedDraft } from "../state/drafts";
import { findThread, useStore } from "../state/store";
import type { Scene } from "./scenes";

const dana: Person = { name: "Dana Whitfield", email: "dana@brightside.agency", initials: "DW", short: "Dana" };
const you: Person = { ...people.maya };

const note = (id: string, from: Person, time: string, sentFull: string, paragraphs: string[], extra: Partial<Message> = {}): Message => ({
  id,
  from,
  to: [from === dana ? you : dana],
  cc: [],
  bcc: [],
  time,
  sentFull,
  paragraphs,
  attachments: [],
  ...extra,
});

/** The conversation the This email frames are about: five earlier messages, then Dana's today. */
export const springCampaign: Thread = {
  id: "spring-campaign",
  accountId: "work",
  split: "important",
  section: "reply",
  from: dana,
  subject: "Spring campaign",
  fullSubject: "Spring campaign: contract and launch plan",
  summary: "Countersign the contract by Thursday 6 November",
  time: "9:12 AM",
  messages: [
    note("spring-1", dana, "Oct 20", "Tuesday, 20 October, 10:02 AM", ["Hi Maya, here’s the brief for the spring campaign. Could you quote for the shoot and the launch visuals?"]),
    note("spring-2", you, "Oct 21", "Wednesday, 21 October, 4:40 PM", ["Thanks Dana. €24,000 for both, half on signing and half after launch."]),
    note("spring-3", dana, "Oct 24", "Saturday, 24 October, 11:15 AM", ["Here’s a first draft of the contract. Legal still has to look at it."], {
      attachments: [{ name: "contract-draft-v1.pdf", size: "280 KB" }],
    }),
    note("spring-4", dana, "Oct 28", "Wednesday, 28 October, 9:30 AM", ["Agreed: €12,000 on signing and €12,000 after launch."]),
    note("spring-5", you, "Oct 28", "Wednesday, 28 October, 2:05 PM", ["Perfect, thank you."]),
    note(
      "spring-6",
      dana,
      "9:12 AM",
      "Sunday, 1 November, 9:12 AM",
      [
        "Hi Maya,",
        "Legal signed off, so the final contract is attached. Could you countersign it and send it back by Thursday 6 November? The budget stays at €24,000: half on signing, half after launch.",
        "Launch moves to Monday 17 November, because the print run needs an extra week. Jonas will send the shot list on Tuesday.",
        "Thanks, Dana",
      ],
      { attachments: [{ name: "Spring-campaign-contract-final.pdf", size: "312 KB" }] },
    ),
  ],
};

/** The older email the All mail answer also points at. */
const firstDraft: Thread = {
  ...springCampaign,
  id: "spring-first-draft",
  subject: "Spring campaign: first contract draft",
  fullSubject: undefined,
  time: "Oct 24",
  messages: [springCampaign.messages[2]],
};

const reply = (id: string, thread: Thread, body: string, extra: Partial<SavedDraft> = {}): SavedDraft => ({
  id,
  mode: "reply",
  threadId: thread.id,
  fromAccountId: thread.accountId,
  to: [thread.from],
  cc: [],
  bcc: [],
  subject: `Re: ${thread.subject}`,
  body,
  bodyHtml: body,
  fieldsOpen: true,
  showCc: false,
  showBcc: false,
  remindIfNoReply: false,
  signatureId: null,
  updatedAt: 0,
  ...extra,
});

const answered = (question: string, text: string, extra: Partial<ChatTurn> = {}): ChatTurn => ({ ...startTurn(question), state: "done", text, ...extra });

/** Puts the sample emails where the panel and the reading view find them. Safe to run twice. */
function withSamples() {
  useStore.setState((s) => ({ doneThreads: [...s.doneThreads.filter((t) => !t.id.startsWith("spring-")), springCampaign, firstDraft] }));
}

/** Opens the panel with `turns` in its chat, over the Spring campaign email or the inbox. */
function ask(over: "email" | "inbox", turns: ChatTurn[], extra: Partial<AskPanelState> = {}) {
  withSamples();
  if (over === "email") {
    // At the top of Important, as in the frames.
    useStore.setState((s) => ({ threads: [springCampaign, ...s.threads.filter((t) => t.id !== springCampaign.id)] }));
    useStore.getState().open(springCampaign.id);
  } else useStore.setState({ view: "inbox" });
  useAskPanel.setState({ open: true, chat: "moss-1", chats: { "moss-1": turns }, input: "", pick: -1, dismissed: [], mailCount: 2431, ...extra });
}

const question = "Find the PDF from the bank";

/** An email whose sender, subject and file are all longer than a card is wide. */
const council: Thread = {
  ...springCampaign,
  id: "council-address",
  from: { name: "Elmford Borough Council Resident Services (Automated Message, Please Do Not Reply)", email: "no-reply@elmford-council.example", initials: "EC" },
  subject: "Elmford Borough Council (Reference EBC-PARK-2207): your request to update the address on your parking permit",
  fullSubject: undefined,
  time: "Wed, Nov 4",
  messages: [
    note("council-1", dana, "Wed, Nov 4", "Wednesday, 4 November, 8:00 AM", ["We received your request to update your address and have changed it on your parking permit."], {
      attachments: [{ name: "EBC-resident-parking-permit-address-update-confirmation-2026-11-04.pdf", size: "88 KB" }],
    }),
  ],
};

/** The letter whose date is only in its PDF. */
const permit: Thread = {
  ...council,
  id: "council-permit",
  subject: "Elmford Borough Council - Letter about your parking permit",
  messages: [
    note("council-2", dana, "Thu, Nov 5", "Thursday, 5 November, 9:00 AM", ["Please see the attached letter."], {
      attachments: [{ name: "P2026-0457.pdf", size: "543 KB" }],
    }),
  ],
};

export const askScenes: Record<string, Scene> = {
  "ask-email-start": { shows: /Ask about this conversation or any other email/, apply: () => ask("email", [], { pick: -1 }) },
  "ask-email-draft": {
    shows: /Nothing sends until you press Send/,
    apply: () =>
      ask("email", [
        answered("Did we agree on the payment split?", "Yes. On Oct 28 Dana agreed to €12,000 on signing and €12,000 after launch. Today’s email keeps that split.", {
          messages: ["spring-4", "spring-6"],
        }),
        answered("Draft a reply: I’ll sign, but ask to move the second payment to December.", "Here’s a draft. Nothing sends until you press Send.", {
          drafts: [
            reply(
              "ask-spring",
              springCampaign,
              "Hi Dana, thanks for the update. I’ll countersign and send the contract back by Thursday. Could the second €12,000 payment come in December instead of after launch? Best, Maya",
            ),
          ],
        }),
      ]),
  },
  "ask-all-cards": {
    shows: /Did Dana send me the contract\?/,
    apply: () =>
      ask(
        "inbox",
        [
          answered(
            "Did Dana send me the contract?",
            "Yes. Dana sent the final contract today at 9:12 AM, after legal signed off. She needs it back, signed, by Thursday 6 November.",
            { sources: [springCampaign, firstDraft] },
          ),
        ],
        { pick: 0 },
      ),
  },
  "ask-all-draft": {
    shows: /Sending doesn’t mark Priya’s email Done/,
    apply: () => {
      withSamples();
      const shoot = findThread(useStore.getState(), "fridays-shoot")!;
      ask("inbox", [
        answered("Tell Priya 10 works for me on Friday", "Here’s a reply to Priya’s email. Check it, then send it or edit it. Nothing sends until you press Send.", {
          sources: [shoot],
          drafts: [reply("ask-shoot", shoot, "Hi Priya, yes, 10 AM on Friday works for me. See you at the studio. Maya", { fromAddress: "hello@rossstudio.art" })],
        }),
      ]);
    },
  },
  "ask-all-long-names": {
    shows: /did the council change my address\?/,
    apply: () => {
      useStore.setState((s) => ({ doneThreads: [...s.doneThreads.filter((t) => t.id !== council.id), council] }));
      ask(
        "inbox",
        [
          answered("did the council change my address?", "Yes. On Nov 4 the council wrote that they changed the address on your parking permit.", { sources: [council] }),
          answered("Thank them", "Here’s a draft. Nothing sends until you press Send.", { drafts: [reply("ask-council", council, "Hello, thank you for updating my address. Best, Maya")] }),
        ],
        { pick: 0 },
      );
    },
  },
  "ask-all-file": {
    shows: /Your new parking permit starts on Thursday 10 December/,
    apply: () => {
      useStore.setState((s) => ({ doneThreads: [...s.doneThreads.filter((t) => t.id !== permit.id), permit] }));
      ask(
        "inbox",
        [
          answered(
            "when does my new parking permit start?",
            "Your new parking permit starts on Thursday 10 December, says the letter the council sent, P2026-0457.pdf.",
            { sources: [permit], files: [{ threadId: permit.id, messageId: "council-2", index: 0, name: "P2026-0457.pdf" }] },
          ),
        ],
        { pick: 0 },
      );
    },
  },
  "ask-acted": {
    shows: /Marked done: Studio notes, issue 42/,
    apply: () =>
      ask("inbox", [
        answered("Mark this week’s newsletters Done", "Done. I marked 3 newsletters Done. They’re in Done if you want one back.", {
          did: ["Marked done: Design weekly", "Marked done: Studio notes, issue 42", "Marked done: Type foundry news"],
        }),
      ]),
  },
  // A model's Markdown, shown as it means: emphasis, a list, a table, code.
  "ask-markdown": {
    shows: /What Dana is waiting on/,
    apply: () =>
      ask("inbox", [
        answered(
          "What is Dana waiting on?",
          "**What Dana is waiting on**\n\nTwo things, both by *Friday*:\n\n1. The countersigned contract for the spring shoot\n2. Your notes on the `Q4-budget.xlsx` numbers\n\n| What | By |\n| --- | --- |\n| Contract | Fri 13 Nov |\n| Budget notes | Fri 13 Nov |\n\n> I can sign off on everything else once these are in.",
        ),
      ]),
  },
  "ask-send-later": {
    shows: /It goes on Monday morning/,
    apply: () => {
      // Next Monday, 9 AM: one to seven days ahead.
      const monday = new Date();
      monday.setDate(monday.getDate() + 7 - ((monday.getDay() + 6) % 7));
      monday.setHours(9, 0, 0, 0);
      const draft = reply("ask-later", springCampaign, "Hi Dana, here’s the countersigned contract. Best, Maya");
      ask("email", [
        answered("Send Dana the signed contract on Monday morning", "Here it is. It goes on Monday morning once you press Send.", {
          drafts: [draft],
          sendAt: { [draft.id]: monday.getTime() },
        }),
      ]);
    },
  },
  "palette-ask": {
    shows: /Ask Moss: “did dana send me the contract”/,
    apply: () => {
      useAskPanel.setState({ open: false });
      useStore.setState({ view: "inbox", paletteOpen: true, paletteQuery: "did dana send me the contract" });
    },
  },
  "ask-thinking": { shows: /Looking through 2,431 emails…/, apply: () => ask("inbox", [startTurn(question)]) },
  "ask-no-model": {
    shows: /Ask needs the writing model/,
    apply: () => ask("inbox", [{ ...startTurn(question), state: "failed", error: { kind: "notReady", message: "the AI model isn’t downloaded yet" } }]),
  },
  "ask-nothing-found": {
    shows: /I couldn’t find a PDF from a bank/,
    apply: () => ask("inbox", [answered(question, "I couldn’t find a PDF from a bank. Closest: a statement from Pebble Bank on Thu, with no attachment.")]),
  },
  "ask-failed": {
    shows: /Something went wrong before I could answer/,
    apply: () => ask("inbox", [{ ...startTurn(question), state: "failed", error: { kind: "model", message: "the AI model failed" } }]),
  },
};
