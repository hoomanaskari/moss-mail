/**
 * Demo mailbox matching design.pen, used until real accounts are connected.
 * Names, subjects and times in the Important split are copied from the design;
 * the other splits are filled to match the design's tab counts.
 */
import type { Account, Person, Split, SplitId, Thread } from "./types";

export const accounts: Account[] = [
  {
    id: "work",
    name: "Work",
    letter: "W",
    address: "maya@northwind.co",
    senderName: "Maya Ross",
    color: "work",
    org: "Northwind",
    orgDomain: "northwind.co",
    connection: { provider: "outlook", summary: "Microsoft 365" },
  },
  {
    id: "personal",
    name: "Personal",
    letter: "P",
    address: "maya.ross@fastmail.com",
    senderName: "Maya Ross",
    color: "personal",
    connection: { provider: "fastmail", summary: "Fastmail · App password" },
  },
  {
    id: "studio",
    name: "Studio",
    letter: "S",
    address: "hello@rossstudio.art",
    senderName: "Maya Ross",
    color: "studio",
    connection: { provider: "resend", summary: "Resend · 2 domains" },
  },
];

/** A fourth account whose sign-in has expired, as drawn in Settings. */
export const gmailDemoAccount: Account = {
  id: "gmail",
  name: "Gmail",
  letter: "G",
  address: "maya.ross@gmail.com",
  senderName: "Maya Ross",
  color: "green",
  connection: { provider: "gmail", summary: "Gmail · Own OAuth client", signedOut: true },
};

export const splits: Split[] = [
  { id: "important", label: "Important" },
  { id: "other", label: "Other" },
  { id: "calendar", label: "Calendar" },
  { id: "newsletters", label: "Newsletters" },
  { id: "receipts", label: "Receipts" },
];

/** More accounts to try the switcher with many inboxes (the many-accounts scenes). */
export const extraDemoAccounts: Account[] = [
  { id: "moss", name: "Moss", letter: "M", address: "hello@moss-mail.example", senderName: "Maya Ross", color: "green" },
  { id: "freelance", name: "Freelance", letter: "F", address: "maya@rossfreelance.dev", senderName: "Maya Ross", color: "violet" },
  { id: "home", name: "Home", letter: "H", address: "maya.ross@icloud.com", senderName: "Maya Ross", color: "sky" },
  { id: "lab", name: "Lab", letter: "L", address: "maya@ross-lab.io", senderName: "Maya Ross", color: "sand" },
  { id: "volunteer", name: "Volunteering", letter: "V", address: "maya@greenstreet.example.org", senderName: "Maya Ross", color: "olive" },
  { id: "prints", name: "Prints", letter: "R", address: "orders@rossprints.shop", senderName: "Maya Ross", color: "work" },
];

const p = (name: string, email: string, extra: Partial<Person> = {}): Person => ({
  name,
  email,
  initials: name
    .split(" ")
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase(),
  short: name.split(" ")[0],
  ...extra,
});

export const people = {
  maya: p("Maya Ross", "maya@northwind.co"),
  daniel: p("Daniel Okafor", "daniel@northwind.co", {
    role: "Finance lead, Northwind",
    location: "Lagos",
    localTime: "10:12 AM",
  }),
  lena: p("Lena Vogt", "lena@northwind.co"),
  jonas: p("Jonas Weber", "jonas@northwind.co"),
  ines: p("Ines Duarte", "ines@duarte-studio.pt"),
  priya: p("Priya Nair", "priya@nairfilms.example"),
  mum: p("Mum", "mum@fastmail.com"),
  financeArchive: p("Finance archive", "finance-archive@northwind.co"),
  financeTeam: p("Finance team", "finance@northwind.co"),
  fintan: p("Fintan Byrne", "fintan.byrne@gmail.com"),
};


/** Your own address in each account, used as the "to" of demo emails. */
const me: Record<string, Person> = {
  work: people.maya,
  personal: p("Maya Ross", "maya.ross@fastmail.com"),
  studio: p("Maya Ross", "hello@rossstudio.art"),
};

const fullTimes: Record<string, string> = {
  Yesterday: "Saturday, 31 October",
  Tomorrow: "Monday, 2 November",
  Fri: "Friday, 30 October",
  Thu: "Thursday, 29 October",
  Wed: "Wednesday, 28 October",
  Mon: "Monday, 26 October",
};

/** "8:40 AM" → "Sunday, 1 November, 8:40 AM"; "Fri" → "Friday, 30 October". */
const fullTime = (time: string) => fullTimes[time] ?? (/AM|PM/.test(time) ? `Sunday, 1 November, ${time}` : time);

interface Extra extends Partial<Thread> {
  /** Paragraphs of the email. */
  body?: string[];
  cc?: Person[];
}

const simple = (
  id: string,
  accountId: string,
  section: Thread["section"],
  from: Person,
  subject: string,
  summary: string | undefined,
  time: string,
  { body, cc = [], ...extra }: Extra = {},
): Thread => ({
  id,
  accountId,
  split: "important",
  section,
  from,
  subject,
  summary,
  time,
  messages: body
    ? [{ id: `${id}-1`, from, to: [me[accountId]], cc, bcc: [], time, sentFull: fullTime(time), paragraphs: body, attachments: [] }]
    : [],
  ...extra,
});

/** A thread outside the Important split. These show as a plain list, styled "for your info". */
const filed = (split: SplitId, id: string, accountId: string, from: Person, subject: string, summary: string, time: string, body: string[]) =>
  simple(id, accountId, "info", from, subject, summary, time, { split, body });

export const threads: Thread[] = [
  {
    id: "q4-budget",
    accountId: "work",
    split: "important",
    section: "reply",
    from: people.daniel,
    subject: "Q4 design budget",
    fullSubject: "Q4 design budget: sign-off needed",
    summary: "Approve the new total, or suggest changes",
    time: "9:12 AM",
    labels: ["Budget", "Approval"],
    ask: "Daniel needs you to approve the €18,400 budget, or say what to change.",
    askDue: "By Thursday",
    recent: [
      { subject: "Planning call notes", when: "Tue" },
      { subject: "Q3 spend review", when: "Oct 14" },
      { subject: "Vendor invoices", when: "Oct 2" },
    ],
    insight: "You usually answer Daniel within a day. He prefers short replies.",
    replyOptions: [
      {
        label: "Approve it",
        draft: "Thanks Daniel, the testing line makes sense. Approved at €18,400. Could you share the testing plan when it’s ready?",
      },
      {
        label: "Ask about testing",
        draft: "Thanks Daniel. Before I approve, could you tell me a bit more about the user testing line? What does it cover, and who is running it?",
      },
      {
        label: "Need more time",
        draft: "Thanks Daniel. I need a little more time to go through this. I’ll get back to you by Thursday morning.",
      },
    ],
    messages: [
      {
        id: "q4-budget-1",
        from: people.daniel,
        to: [people.maya, people.lena],
        cc: [people.jonas, people.ines],
        bcc: [],
        time: "9:12 AM",
        sentFull: "Sunday, 1 November, 9:12 AM",
        paragraphs: [
          "Hi Maya,",
          "I’ve updated the Q4 design budget after Tuesday’s planning call. The main change is a new line for user testing, which we agreed is worth doing before the onboarding launch.",
          "Could you approve it by Thursday? If anything looks off, reply with what you’d change.",
          "Thanks, Daniel",
        ],
        attachments: [{ name: "Q4-design-budget-v2.pdf", size: "240 KB" }],
      },
    ],
  },
  simple("fridays-shoot", "studio", "reply", people.priya, "Friday’s shoot", "Can it start at 10 AM? Yes or no.", "8:40 AM", {
    body: [
      "Hi Maya,",
      "The client asked if we can start Friday’s shoot at 10 AM instead of 9. The light in the studio is still good then, and it gives the stylist more time.",
      "Does 10 work for you? A yes or no is all I need.",
      "Priya",
    ],
    between: { emails: 6, since: "March", youReply: "Same day" },
    recent: [
      { subject: "Studio day rates", when: "Oct 21" },
      { subject: "Moodboard v2", when: "Oct 9" },
    ],
  }),
  simple("sunday-lunch", "personal", "reply", people.mum, "Sunday lunch", "Are you coming, and can you bring dessert?", "Yesterday", {
    body: [
      "Hi love,",
      "Are you coming for lunch on Sunday? Your brother is bringing the kids, so it’s around 1 o’clock.",
      "Could you bring dessert? Nothing fancy. The lemon tart from last time was lovely.",
      "Mum x",
    ],
  }),
  simple("onboarding-v3", "work", "reply", people.lena, "Onboarding flow v3", "3 comments to answer", "Yesterday", {
    due: "Due Wed",
    body: [
      "Hi Maya,",
      "I left 3 comments on Onboarding flow v3 in Framely. Two are small copy changes. The third is about skipping the welcome screen for invited users.",
      "Could you answer them by Wednesday? Engineering starts building on Thursday.",
      "Thanks, Lena",
    ],
  }),
  simple("flight-booking", "personal", "info", p("Gull Air", "noreply@gullair.example"), "Booking confirmed", "Fri 13 Nov, 7:25 AM to Lisbon", "Oct 28", {
    body: [
      "Your booking is confirmed.",
      "TP 1351, London Heathrow to Lisbon. Friday 13 November, leaving 7:25 AM, arriving 10:05 AM. Seat 14A.",
      "Online check-in opens 36 hours before departure.",
    ],
  }),
  simple("office-closed", "work", "info", p("Northwind HR", "hr@northwind.co"), "Office closed 11 Nov", "No action needed", "Fri", {
    body: [
      "Hi everyone,",
      "The office will be closed on Wednesday 11 November for building maintenance. Please work from home that day.",
      "There’s nothing you need to do.",
      "Northwind HR",
    ],
  }),
  simple("framely-digest", "work", "info", p("Framely", "no-reply@framely.example"), "Comment digest", "5 new comments on Onboarding v3", "Fri", {
    body: [
      "5 new comments on Onboarding flow v3.",
      "Lena Vogt left 3 comments, Jonas Weber left 1, and Daniel Okafor left 1.",
      "Open the file to reply.",
    ],
  }),
  simple("pebble-statement", "personal", "info", p("Pebble Bank", "help@pebblebank.example"), "October statement", "Nothing is due", "Thu", {
    body: [
      "Your October statement is ready.",
      "You spent £1,284.60 this month. Your bills are all paid and nothing is due.",
      "You can download the statement in the app.",
    ],
  }),
  simple("contract-renewal", "work", "later", people.jonas, "Contract renewal", undefined, "2:00 PM", {
    body: [
      "Hi Maya,",
      "Your freelance contract with Northwind ends on 30 November. I’d like to renew it for another 6 months on the same terms.",
      "Can we talk about it this week?",
      "Jonas",
    ],
  }),
  simple("opening-night", "studio", "later", p("Gallery Nord", "hello@gallerynord.example"), "Opening night list", undefined, "Tomorrow", {
    body: [
      "Hello Maya,",
      "We’re finalising the guest list for the opening night on 20 November. You have 4 places.",
      "Please send us the names of your guests by Friday.",
      "Gallery Nord",
    ],
  }),

  // Other: updates that don't need you (2 work, 1 personal, 1 studio)
  filed("other", "notion-shared", "work", p("Northwind Wiki", "wiki@northwind.co"), "3 pages shared with you", "Q4 roadmap, Launch checklist, Research notes", "9:30 AM", [
    "Lena Vogt shared 3 pages with you: Q4 roadmap, Launch checklist and Research notes.",
    "You can view and edit all of them.",
  ]),
  filed("other", "drive-storage", "work", p("Northwind IT", "it@northwind.co"), "Laptop update on Thursday", "Restart before you leave", "Fri", [
    "Hi Maya,",
    "We’re installing a security update on all laptops on Thursday night. Please save your work and restart before you leave.",
    "Northwind IT",
  ]),
  filed("other", "fastmail-signin", "personal", p("Fastmail", "support@fastmail.com"), "New sign-in", "Safari on Mac, Lisbon", "Thu", [
    "We noticed a new sign-in to your account from Safari on Mac, in Lisbon.",
    "If this was you, there’s nothing to do. If not, change your password.",
  ]),
  filed("other", "site-visits", "studio", p("Studio site", "reports@rossstudio.art"), "Weekly visits", "1,240 visits, up 12%", "Mon", [
    "Your site had 1,240 visits this week, up 12% from last week.",
    "Most visitors came to the Autumn portfolio page.",
  ]),

  // Calendar (1 personal)
  filed("calendar", "checkup", "personal", p("Riverside Dental", "appointments@riverside-dental.example"), "Invitation: Check-up", "Thu 12 Nov, 3:00 PM", "Wed", [
    "You’re invited to a check-up on Thursday 12 November at 3:00 PM.",
    "Riverside Dental, 14 Mill Lane. Please arrive 5 minutes early.",
  ]),

  // Newsletters (6 work, 2 personal, 1 studio)
  filed("newsletters", "design-weekly", "work", p("Design Weekly", "hello@designweekly.example"), "Issue 212: Type that reads well", "Line length, spacing and contrast", "8:00 AM", [
    "This week: how line length, spacing and contrast make long text easier to read.",
    "Plus: 5 new open-source typefaces and a guide to variable fonts.",
  ]),
  filed("newsletters", "product-notes", "work", p("Product Notes", "notes@productnotes.example"), "Saying no to features", "A short guide for small teams", "Yesterday", [
    "How small teams decide what not to build, with 3 examples from real roadmaps.",
  ]),
  filed("newsletters", "northwind-digest", "work", p("Northwind Digest", "digest@northwind.co"), "This week at Northwind", "New hires, launch dates, Friday lunch", "Fri", [
    "Welcome to 3 new people in Design and Engineering. The onboarding launch moves to 24 November. Friday lunch is Lebanese.",
  ]),
  filed("newsletters", "the-interface", "work", p("The Interface", "letters@the-interface.example"), "Calm software", "Why fewer notifications win", "Thu", [
    "Apps that interrupt less get used more. This issue looks at 4 products that cut notifications and grew.",
  ]),
  filed("newsletters", "css-notes", "work", p("CSS Notes", "hi@cssnotes.example"), "Container queries in practice", "Patterns you can use today", "Wed", [
    "Five layout patterns that are simpler with container queries, with code you can copy.",
  ]),
  filed("newsletters", "research-roundup", "work", p("Research Roundup", "team@researchroundup.example"), "Testing with 5 users", "Still enough in 2026?", "Mon", [
    "We look at whether testing with 5 users still finds most problems, and when you need more.",
  ]),
  filed("newsletters", "lisbon-eats", "personal", p("Lisbon Eats", "hello@lisboneats.example"), "Where to eat in November", "6 new places in Alfama", "Sat", [
    "Six new places to try in Alfama this month, from a tiny fish grill to a late-night pastry counter.",
  ]),
  filed("newsletters", "slow-weekend", "personal", p("The Slow Weekend", "letters@slowweekend.example"), "A walk, a book, a long lunch", "Ideas for a quiet weekend", "Fri", [
    "Three gentle plans for a quiet weekend, and the book we can’t stop recommending.",
  ]),
  filed("newsletters", "nord-autumn", "studio", p("Gallery Nord", "news@gallerynord.example"), "Autumn openings", "4 shows this season", "Thu", [
    "Four new shows open this autumn, including a group show of young Lisbon photographers.",
  ]),

  // Receipts (3 personal)
  filed("receipts", "bookshop-order", "personal", p("Bairro Books", "orders@bairrobooks.example"), "Your order has shipped", "2 books, arriving Tuesday", "Yesterday", [
    "Your order of 2 books has shipped and should arrive on Tuesday.",
    "Total paid: €38.40.",
  ]),
  filed("receipts", "flight-receipt", "personal", p("Gull Air", "noreply@gullair.example"), "Receipt for your booking", "€142.80, booking 7XK2LM", "Oct 28", [
    "Thank you for your payment of €142.80 for booking 7XK2LM.",
  ]),
  filed("receipts", "grocer-receipt", "personal", p("Corner Grocer", "receipts@corner-grocer.example"), "Your receipt", "£34.20, Saturday shop", "Sat", [
    "Thanks for shopping with us. You paid £34.20 on Saturday 31 October.",
  ]),
];

/** Minutes before now, so the Done list always reads "Today", "Yesterday"… */
const ago = (minutes: number) => Date.now() - minutes * 60_000;

/** The Done list: two emails done today, and a few from before. */
export const doneThreads: Thread[] = [
  simple("standup-notes", "work", "info", people.jonas, "Standup notes", "Nothing blocking this week", "8:40 AM", {
    body: ["Hi all,", "Nothing blocking this week. The research notes are in the shared folder.", "Jonas"],
    doneAt: ago(25),
  }),
  simple("dinner-sunday", "personal", "reply", people.mum, "Dinner on Sunday?", "Roast at 1, bring nothing", "8:05 AM", {
    body: ["Hello love,", "Roast at 1 on Sunday. Bring nothing, just yourself.", "Mum x"],
    doneAt: ago(70),
  }),
  simple("print-proofs", "studio", "reply", people.ines, "Print proofs", "Approved, sending to print", "Sat", {
    body: ["Hi Maya,", "The proofs look great. Sending them to print on Monday.", "Ines"],
    doneAt: ago(60 * 24 + 30),
  }),
  {
    ...filed("receipts", "train-tickets", "personal", p("Railway Passes", "tickets@railpasses.example"), "Your tickets to Lisbon", "2 adults, 14 Nov", "Thu", [
      "Your tickets are attached. Show them on your phone at the gate.",
    ]),
    doneAt: ago(60 * 24 * 3),
  },
  simple("q3-review", "work", "info", people.daniel, "Q3 spend review", "Final numbers attached", "Oct 14", {
    body: ["Hi Maya,", "Here are the final Q3 numbers. No action needed.", "Daniel"],
    doneAt: ago(60 * 24 * 12),
  }),
];

const jordan = p("Jordan Blake", "jordan@growthleads.example");
const quickQuestion = [
  "Hi there,",
  "I came across Ross Studio and loved your recent work. We help creative studios get three times more leads with done-for-you outreach.",
  "Would you be open to a 15-minute call this week?",
  "Jordan",
];

/** A first email from someone Maya has never written to: Moss asks "Block this sender?". */
export const newSenderThread: Thread = simple(
  "quick-question",
  "studio",
  "reply",
  jordan,
  "Quick question about Ross Studio",
  "Would you be open to a 15-minute call this week?",
  "7:12 AM",
  { body: quickQuestion, sender: "new", between: { emails: 1, since: "Today", youWrote: "Never" } },
);

/** Mail from senders Maya blocked, newest first, as it sits in each account's Blocked folder. */
export const blockedThreads: Thread[] = [
  { ...newSenderThread, sender: "blocked", lastMessageAt: ago(10) },
  simple("promo-sale", "work", "info", p("PromoMail Deals", "deals@promomail.example"), "48 hours only: 70% off everything", "Your code expires soon", "6:30 AM", {
    body: ["Your exclusive code expires soon. Shop now and save 70% on everything."],
    sender: "blocked",
    lastMessageAt: ago(20),
  }),
  simple("following-up", "studio", "reply", jordan, "Following up", "Bumping this to the top of your inbox", "Sat", {
    body: ["Hi again,", "Bumping this to the top of your inbox. Any thoughts?", "Jordan"],
    sender: "blocked",
    lastMessageAt: ago(60 * 24 * 2),
  }),
  simple("seo-ranking", "studio", "reply", p("Alex Rivera", "alex@seo-boost.example"), "Your website’s Google ranking", "I noticed a few issues on rossstudio.art", "Oct 22", {
    body: ["Hi,", "I noticed a few issues on rossstudio.art that are hurting your Google ranking.", "Alex"],
    sender: "blocked",
    lastMessageAt: ago(60 * 24 * 11),
  }),
  simple("bonus-claim", "personal", "info", p("Vault Rewards", "rewards@vault-bonus.example"), "Claim your bonus now", "Your account has an unclaimed reward", "Oct 15", {
    body: ["Your account has an unclaimed reward. Claim it before it expires."],
    sender: "blocked",
    lastMessageAt: ago(60 * 24 * 18),
  }),
  simple("directory-listing", "studio", "reply", p("Hannah Lee", "hannah@bizlistings.example"), "Partnership opportunity", "We’d love to list Ross Studio in our directory", "Oct 9", {
    body: ["Hi Maya,", "We’d love to list Ross Studio in our directory of creative studios.", "Hannah"],
    sender: "blocked",
    lastMessageAt: ago(60 * 24 * 24),
  }),
];

/** "Today" for the demo mailbox, so search can tell this week from last month. */
export const demoToday = "2026-11-01";

const daniel = (id: string, subject: string, summary: string, when: string, date: string, relevance: number, hasAttachment = false) => ({
  id,
  accountId: "work",
  from: people.daniel,
  subject,
  summary,
  when,
  date,
  relevance,
  hasAttachment,
});

/** What the search index holds, as the Search results design shows it for "budget". */
export const searchIndex: import("../lib/search").SearchDoc[] = [
  { ...daniel("s-q4", "Q4 design budget: sign-off needed", "Approve the new budget total of €18,400", "9:12 AM", "2026-11-01", 0.95, true), threadId: "q4-budget" },
  daniel("s-q3", "Q3 spend review", "Came in 4% under budget", "Oct 14", "2026-10-14", 0.85),
  daniel("s-template", "Budget template for 2027", "Use this for next year’s plan", "Sep 30", "2026-09-30", 0.9, true),
  daniel("s-planning", "Planning call notes", "Budget is item 2 on the list", "Tue", "2026-10-27", 0.5),
  daniel("s-vendors", "Re: Testing vendors", "Two quotes, both within budget", "Mon", "2026-10-26", 0.5, true),
  daniel("s-invoices", "Vendor invoices", "Check them against the budget before paying", "Oct 2", "2026-10-02", 0.4),
  daniel("s-offsite", "Offsite dinner", "Keep it under the team budget", "Oct 1", "2026-10-01", 0.4),
  { id: "s-shoot", threadId: "fridays-shoot", accountId: "studio", from: people.priya, subject: "Friday’s shoot", summary: "Can it start at 10 AM? Yes or no.", when: "8:40 AM", date: "2026-11-01", relevance: 0.6, hasAttachment: false },
  { id: "s-onboarding", threadId: "onboarding-v3", accountId: "work", from: people.lena, subject: "Onboarding flow v3", summary: "3 comments to answer, budget for testing agreed", when: "Sat", date: "2026-10-31", relevance: 0.5, hasAttachment: false },
  { id: "s-lunch", threadId: "sunday-lunch", accountId: "personal", from: people.mum, subject: "Sunday lunch", summary: "Are you coming, and can you bring dessert?", when: "Sat", date: "2026-10-31", relevance: 0.5, hasAttachment: false },
];
