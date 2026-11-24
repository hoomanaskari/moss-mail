/**
 * Real-world shaped emails for the `?scene=` states that show HTML and plain-text mail:
 * a designed newsletter with tracking pixels and its own dark version, a light-only receipt,
 * dark styles hung from a class on <html>, a dark version that leaves its text dark,
 * a Gmail reply with quoted history and a plain-text note.
 * Remote addresses use `.example` so nothing real is ever contacted.
 */
import { accounts, people } from "../data/demo";
import type { Message, Person, Thread } from "../data/types";

const harbourBikes: Person = { name: "Harbour Bikes", email: "orders@harbourbikes.example", initials: "HB" };
const fernway: Person = { name: "Fernway", email: "news@fernway.example", initials: "FW" };
const kettle: Person = { name: "Kettle & Co", email: "bookings@kettle.example", initials: "KC" };
const slowGarden: Person = { name: "The Slow Garden", email: "letters@slowgarden.example", initials: "SG" };

const newsletterHtml = `<!doctype html>
<html><head>
<meta http-equiv="refresh" content="0;url=https://slowgarden.example/track">
<style>
  .btn { display: inline-block; padding: 12px 22px; border-radius: 6px; background: #2f5d50; color: #ffffff !important; text-decoration: none; font-weight: bold; }
  .muted { color: #7a7266; font-size: 12px; }
  @media (prefers-color-scheme: dark) {
    .wrap { background: #12100e !important; color: #e8e2d9 !important; }
    .card { background: #1d1a17 !important; }
    .head { border-bottom-color: #2e2a25 !important; }
    .eyebrow { color: #d4b06a !important; }
    .title { color: #f3efe8 !important; }
    .foot { background: #171513 !important; }
    .muted, .muted a { color: #9a9185 !important; }
    .btn { background: #8cbfae; color: #10201b !important; }
  }
</style></head>
<body bgcolor="#efe9df" style="margin:0; font-family: Georgia, serif; color: #2b2620;">
<img src="https://t.slowgarden.example/open.gif?u=8812" width="1" height="1" alt="">
<table width="100%" bgcolor="#efe9df" cellpadding="0" cellspacing="0" class="wrap"><tr><td align="center" style="padding: 28px 12px;">
  <table width="600" cellpadding="0" cellspacing="0" bgcolor="#ffffff" style="max-width: 600px;" class="card">
    <tr><td style="padding: 22px 32px; border-bottom: 1px solid #e6dfd2;" class="head">
      <img src="https://images.slowgarden.example/logo.png" width="140" height="32" alt="The Slow Garden">
    </td></tr>
    <tr><td><img src="https://images.slowgarden.example/autumn-hero.jpg" width="600" height="260" alt="Seed heads in low autumn light"></td></tr>
    <tr><td style="padding: 28px 32px 8px;">
      <p class="eyebrow" style="margin:0 0 6px; color:#8a6d3b; font: bold 12px Arial, sans-serif; letter-spacing: 1.5px;">LETTER 41 · NOVEMBER</p>
      <h1 class="title" style="margin:0 0 14px; font-size: 28px; line-height: 1.25; color:#1f2a24;">Leave the seed heads standing</h1>
      <p style="margin:0 0 14px; font-size: 16px; line-height: 1.6;">Before you tidy the borders this month, look again at what is left. Teasel, coneflower and fennel feed finches all winter, and the hollow stems shelter the insects your spring garden will need.</p>
      <p style="margin:0 0 22px; font-size: 16px; line-height: 1.6;">This week: three plants worth leaving, and the one job you should still do before the frost.</p>
      <a class="btn" href="https://slowgarden.example/letters/41?utm_source=email">Read the full letter</a>
    </td></tr>
    <tr><td style="padding: 26px 32px;">
      <table width="100%" cellpadding="0" cellspacing="0"><tr>
        <td width="50%" valign="top" style="padding-right: 10px;">
          <img src="https://images.slowgarden.example/teasel.jpg" width="258" height="160" alt="Teasel">
          <p style="font: bold 15px Georgia, serif; margin: 10px 0 4px;">Teasel</p>
          <p style="font-size: 14px; line-height: 1.5; margin: 0;">Goldfinches pick the seeds out one by one.</p>
        </td>
        <td width="50%" valign="top" style="padding-left: 10px;">
          <img src="https://images.slowgarden.example/fennel.jpg" width="258" height="160" alt="Bronze fennel">
          <p style="font: bold 15px Georgia, serif; margin: 10px 0 4px;">Bronze fennel</p>
          <p style="font-size: 14px; line-height: 1.5; margin: 0;">Ladybirds overwinter in the stems.</p>
        </td>
      </tr></table>
    </td></tr>
    <tr><td bgcolor="#f7f3ec" style="padding: 20px 32px;" class="muted foot">
      You get this because you subscribed at slowgarden.example.
      <a href="https://slowgarden.example/unsubscribe?u=8812" style="color:#7a7266;">Unsubscribe</a>
      <img src="https://t.slowgarden.example/footer.gif?u=8812" width="1" height="1" alt="">
    </td></tr>
  </table>
</td></tr></table>
<script>document.location = "https://slowgarden.example/track"</script>
</body></html>`;

const receiptHtml = `<!doctype html>
<html><head><style>
  .row td { padding: 10px 0; border-bottom: 1px solid #ececec; font-size: 15px; }
</style></head>
<body style="margin:0; background:#f2f2f2; font-family: Helvetica, Arial, sans-serif; color:#1a1a1a;">
<table width="100%" bgcolor="#f2f2f2" cellpadding="0" cellspacing="0"><tr><td align="center" style="padding: 24px 12px;">
  <table width="520" cellpadding="0" cellspacing="0" bgcolor="#ffffff" style="max-width: 520px;">
    <tr><td bgcolor="#e4572e" style="padding: 18px 28px; color:#ffffff; font: bold 20px Helvetica, Arial, sans-serif;">Harbour Bikes</td></tr>
    <tr><td style="padding: 24px 28px;">
      <p style="margin:0 0 4px; font-size: 22px; font-weight: bold;">Thanks for your order</p>
      <p style="margin:0 0 18px; color:#6b6b6b; font-size: 14px;">Order HB-20417 · 1 November 2026</p>
      <table width="100%" cellpadding="0" cellspacing="0">
        <tr class="row"><td>Rear light, USB</td><td align="right">€24.00</td></tr>
        <tr class="row"><td>Chain oil, 100 ml</td><td align="right">€8.50</td></tr>
        <tr><td style="padding: 14px 0 0; font-weight: bold;">Total</td><td align="right" style="padding: 14px 0 0; font-weight: bold;">€32.50</td></tr>
      </table>
    </td></tr>
  </table>
</td></tr></table>
</body></html>`;

/** Dark styles that hang from a class on <html>, as some email builders write them. */
const canvasHtml = `<!doctype html>
<html class="builder-canvas" lang="en"><head>
<meta name="color-scheme" content="light dark">
<style>
  @media (prefers-color-scheme: dark) {
    .builder-canvas .color-fg { color: rgb(255, 255, 255) !important; }
    .builder-canvas .color-muted { color: rgb(138, 145, 158) !important; }
    .builder-canvas .bg-bg { background-color: rgb(10, 11, 13) !important; }
    .builder-canvas .bg-alt { background-color: rgb(20, 21, 25) !important; }
  }
</style></head>
<body class="email-body" style="margin:0; font-family: Helvetica, Arial, sans-serif;">
<table width="100%" cellpadding="0" cellspacing="0"><tr><td align="center" style="padding: 24px 12px;">
  <table width="560" cellpadding="0" cellspacing="0" style="max-width: 560px; border: 1px solid #d9dbde; border-radius: 24px; overflow: hidden;">
    <tr><td class="bg-bg" style="background-color: #ffffff; padding: 22px 40px; color: #0052ff; font: bold 24px Helvetica, Arial, sans-serif;">Fernway</td></tr>
    <tr><td bgcolor="#3773f5" style="padding: 48px 40px; color: #ffffff; font: bold 56px Helvetica, Arial, sans-serif;">3%</td></tr>
    <tr><td class="bg-bg" style="background-color: #ffffff; padding: 28px 40px;">
      <h1 class="color-fg" style="margin: 0 0 16px; font-size: 28px; color: rgb(10, 11, 13);">A bonus on every transfer</h1>
      <p class="color-fg" style="margin: 0 0 16px; font-size: 16px; line-height: 24px; color: rgb(10, 11, 13);">Move savings to Fernway before 15 November and get 3% back on eligible transfers.</p>
      <p class="color-fg" style="margin: 0; font-size: 16px; line-height: 24px; color: rgb(10, 11, 13);">The bonus is paid within five working days after the offer ends.</p>
    </td></tr>
    <tr><td class="bg-alt color-muted" style="background-color: #eef0f3; padding: 20px 40px; font-size: 12px; color: rgb(91, 97, 110);">Fernway Ltd · fernway.example · Unsubscribe</td></tr>
  </table>
</td></tr></table>
</body></html>`;

/** A dark version only half done: it darkens the paper and leaves the text dark. */
const halfDarkHtml = `<!doctype html>
<html><head><style>
  @media (prefers-color-scheme: dark) {
    .page, .card { background-color: #121212 !important; }
  }
</style></head>
<body style="margin:0; font-family: Helvetica, Arial, sans-serif;">
<table width="100%" cellpadding="0" cellspacing="0" class="page" style="background-color: #f2f2f2;"><tr><td align="center" style="padding: 24px 12px;">
  <table width="520" cellpadding="0" cellspacing="0" class="card" style="max-width: 520px; background-color: #ffffff;">
    <tr><td style="padding: 18px 28px; background-color: #1f6f5c; color: #ffffff; font: bold 20px Helvetica, Arial, sans-serif;">Kettle &amp; Co</td></tr>
    <tr><td style="padding: 24px 28px; color: #1a1a1a;">
      <p style="margin: 0 0 12px; font-size: 22px; font-weight: bold;">Your table is booked</p>
      <p style="margin: 0 0 12px; font-size: 15px; line-height: 22px;">Saturday 14 November at 19:30, for four. We'll keep the table for 15 minutes.</p>
      <p style="margin: 0; font-size: 15px; line-height: 22px;">Need to change it? Reply to this email or call us on 01632 960 123.</p>
    </td></tr>
  </table>
</td></tr></table>
</body></html>`;

const replyHtml = `<div dir="ltr"><div style="font-family: Arial, Helvetica, sans-serif; color: #222222; font-size: small;">Hi Maya,<br><br>Friday at 10 is perfect, thank you for moving things around. I've put the call sheet here: <a href="https://nairfilms.example/callsheet/fri">nairfilms.example/callsheet/fri</a><br><br>Two small things:<ul><li>Could the client bring the <b>autumn samples</b>?</li><li>We'll need the studio from 9:30 to set up.</li></ul>See you then,<br>Priya</div></div><br>
<div class="gmail_quote"><div dir="ltr" class="gmail_attr">On Sun, 1 Nov 2026 at 09:40, Maya Ross &lt;<a href="mailto:hello@rossstudio.art">hello@rossstudio.art</a>&gt; wrote:<br></div>
<blockquote class="gmail_quote" style="margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex">Hi Priya,<br><br>10 AM on Friday works for me. I'll let the client know the new time.<br><br>See you then,<br>Maya</blockquote></div>`;

const plainText = `Hi Maya,

The photos from Saturday are up here: https://photos.example/album/harbour-walk

I've also put the route on the map in case you want to do it again:
www.maps.example/route/8812

Fintan

On Sat, 31 Oct 2026 at 18:02, Maya Ross <maya.ross@fastmail.com> wrote:
> That was lovely, thank you for organising.
> Could you share the photos when you get a chance?
>
> Maya
`;

const msg = (id: string, from: Person, to: Person, body: Partial<Message>, paragraphs: string[]): Message => ({
  id,
  from,
  to: [to],
  cc: [],
  bcc: [],
  time: "8:05 AM",
  sentFull: "Sunday, 1 November, 8:05 AM",
  paragraphs,
  attachments: [],
  ...body,
});

const [, personal, studio] = accounts;

export const sampleThreads: Thread[] = [
  {
    id: "sample-newsletter",
    accountId: personal.id,
    split: "newsletters",
    section: "info",
    from: slowGarden,
    subject: "Leave the seed heads standing",
    time: "8:05 AM",
    labels: ["Newsletter"],
    messages: [msg("sample-newsletter-1", slowGarden, people.maya, { html: newsletterHtml }, ["Leave the seed heads standing."])],
  },
  {
    id: "sample-receipt",
    accountId: personal.id,
    split: "other",
    section: "info",
    from: harbourBikes,
    subject: "Your Harbour Bikes order HB-20417",
    time: "9:12 AM",
    messages: [msg("sample-receipt-1", harbourBikes, people.maya, { html: receiptHtml }, ["Thanks for your order."])],
  },
  {
    id: "sample-canvas",
    accountId: personal.id,
    split: "other",
    section: "info",
    from: fernway,
    subject: "A bonus on every transfer",
    time: "11:02 AM",
    messages: [msg("sample-canvas-1", fernway, people.maya, { html: canvasHtml }, ["A bonus on every transfer."])],
  },
  {
    id: "sample-half-dark",
    accountId: personal.id,
    split: "other",
    section: "info",
    from: kettle,
    subject: "Your table is booked",
    time: "11:40 AM",
    messages: [msg("sample-half-dark-1", kettle, people.maya, { html: halfDarkHtml }, ["Your table is booked."])],
  },
  {
    id: "sample-reply",
    accountId: studio.id,
    split: "important",
    section: "reply",
    from: people.priya,
    subject: "Re: Friday’s shoot",
    time: "10:20 AM",
    messages: [msg("sample-reply-1", people.priya, { ...people.maya, email: studio.address }, { html: replyHtml }, ["Friday at 10 is perfect."])],
  },
  {
    id: "sample-text",
    accountId: personal.id,
    split: "other",
    section: "info",
    from: people.fintan,
    subject: "Re: Photos from the harbour walk",
    time: "Yesterday",
    messages: [msg("sample-text-1", people.fintan, { ...people.maya, email: personal.address }, { text: plainText }, ["The photos are up."])],
  },
];

