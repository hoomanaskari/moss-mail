//! The decision model on where mail goes: is mail sent with mailing-list headers a newsletter,
//! or a notice about the user's own work or account? Is a letter from a stranger a sales pitch
//! (Newsletters) or a real person (Important)? Against the real model on this Mac. Ignored by
//! default: it needs the downloaded decision model.
//!
//! ```sh
//! MOSS_AI_DIR="$HOME/Library/Application Support/app.moss.mail/ai" \
//! cargo test -p moss-ai --test real_split -- --ignored --nocapture
//! ```
//!
//! Every email here is made up. Only `Tune` mail was used to choose the questions' wording;
//! `Held`, `Fresh` and `Late` mail only score it (`Fresh` was written once the list question's
//! wording was fixed, `Late` once the pitch questions' were). Keep it that way: a wording change
//! is judged on the other halves, never fitted to them. `Held` was scored once before the
//! second pitch question (`prompt::wants`) was added for a kind of mistake it showed, so `Late`
//! is the clean check of that.
//!
//! `MOSS_LIST_MAIL=<file.json>` also sorts a JSON list of emails (see [`Listed`]) and prints
//! only how many of each kind land in each tab, before and now.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use moss_ai::manager::{free_space, Catalog, Llama};
use moss_ai::{prompt, Ai, Machine, Parts};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    /// About the user's own work or account: should not be in Newsletters.
    Notice,
    /// Sent to many: belongs in Newsletters.
    Newsletter,
    /// A sales, recruiting, funding or partnership pitch from a stranger, or a follow-up to
    /// one: marketing, so Newsletters.
    Pitch,
    /// A real person writing for the first time: Important, never Newsletters.
    Person,
    /// Someone the user has written to: Important.
    Contact,
}
use Kind::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Half {
    Tune,
    Held,
    Fresh,
    /// Written once the pitch questions were fixed, after `Held` had been scored once.
    Late,
}
use Half::*;

/// How the mail came, so where the rules put it before the model is asked (the store's
/// `classify::split_for`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Via {
    /// With mailing-list headers: Newsletters.
    List,
    /// From a person's address, no list headers: Important.
    Direct,
    /// From a no-reply or helpdesk address, or marked automatic: Other.
    Robot,
}
use Via::*;

struct Mail {
    kind: Kind,
    half: Half,
    via: Via,
    from: &'static str,
    subject: &'static str,
    body: &'static str,
}

/// Mail that came with list headers.
const fn m(kind: Kind, half: Half, from: &'static str, subject: &'static str, body: &'static str) -> Mail {
    Mail { kind, half, via: List, from, subject, body }
}

/// Mail from a person's address, without list headers.
const fn d(kind: Kind, half: Half, from: &'static str, subject: &'static str, body: &'static str) -> Mail {
    Mail { kind, half, via: Direct, from, subject, body }
}

/// Mail from an automatic sender.
const fn r(kind: Kind, half: Half, from: &'static str, subject: &'static str, body: &'static str) -> Mail {
    Mail { kind, half, via: Robot, from, subject, body }
}

const MAIL: &[Mail] = &[
    // Notifications, tuning half.
    m(Notice, Tune, "Codeforge <notifications@codeforge.example>", "Re: [acme/widgets] Add retry to the uploader (PR #412)",
      "lintbot[bot] commented on this pull request.\n\nIn src/upload.rs line 88:\n> let tries = 3;\nConsider reading the retry count from the config so it can be changed without a release.\n\nReply to this email directly, view it on Codeforge, or unsubscribe.\nYou are receiving this because you authored the thread."),
    m(Notice, Tune, "Pipewright CI <builds@pipewright.example>", "[widgets] Build #2291 failed on main",
      "Build #2291 failed.\n\nCommit 3f2a1c9 by Maya Ross: Add retry to the uploader\nJob test-linux exited with status 1 after 4m 12s.\n\nFailing test: upload::retries_three_times\n\nView the build log. You get these emails because you pushed this commit. Manage notifications."),
    m(Notice, Tune, "Trackly <notify@trackly.example>", "Priya Nair mentioned you in WID-88: Uploader times out on large files",
      "Priya Nair mentioned you in a comment:\n\n@maya can you take a look at the logs from last night? The timeout happens after exactly 30 seconds, so I think it is the proxy.\n\nView issue WID-88. Turn off mention emails in your notification settings."),
    m(Notice, Tune, "Lumen Cloud <security@lumencloud.example>", "A passkey was added to your Lumen Cloud account",
      "Hi Maya,\n\nA new passkey called Work laptop was added to your Lumen Cloud account on 14 Nov 2026 at 22:41 UTC.\n\nIf you added it, you can ignore this email. If not, remove it and review your security settings now.\n\nLumen Cloud Security. Unsubscribe from security tips."),
    m(Notice, Tune, "Papyr <comments@papyr.example>", "Tom Okafor commented on \"Q4 roadmap\"",
      "Tom Okafor commented on Q4 roadmap:\n\n\"Can we move the billing work to January? The team is stretched in December.\"\n\nOn the text: Billing revamp, target 15 December\n\nReply to the comment or open the document. You are receiving this because you own this document."),
    m(Notice, Tune, "Fernwood Community <forum@fernwood.example>", "New reply to your post: Battery drains overnight",
      "jlee replied to your topic Battery drains overnight:\n\nI had the same thing on the Fernwood One. Turning off background sync in Settings fixed it for me. Which firmware are you on?\n\nVisit the topic to reply. You are getting this because you are watching this topic. Unsubscribe from this topic."),
    m(Notice, Tune, "Shipyard <deploys@shipyard.example>", "Deploy succeeded: widgets-api to production",
      "Your deploy finished.\n\nApp: widgets-api\nEnvironment: production\nRelease: v2.14.0 (3f2a1c9)\nStarted by: Maya Ross\nDuration: 2m 37s\n\nAll health checks passed. View the release or roll back from the dashboard."),
    m(Notice, Tune, "Huddle <reminders@huddle.example>", "Your 1:1 with Lena Park is at 15:00",
      "Your 1:1 with Lena Park begins at 15:00 today.\n\nAgenda: export dialog, Q1 goals\nJoin from the calendar event or the link below.\n\nYou get these because reminders are on for your meetings. Change your reminder settings."),
    // Notifications, held out.
    m(Notice, Held, "Codeforge <notifications@codeforge.example>", "[acme/widgets] Bump serde from 1.0.219 to 1.0.220 (PR #415)",
      "depbot[bot] requested your review on: acme/widgets#415 Bump serde from 1.0.219 to 1.0.220.\n\nBumps serde from 1.0.219 to 1.0.220. Release notes and commits are linked in the pull request.\n\nReply to this email directly, view it on Codeforge, or unsubscribe.\nYou are receiving this because your review was requested."),
    m(Notice, Held, "Codeforge <notifications@codeforge.example>", "[acme/widgets] Critical security alert for left-trim",
      "A critical vulnerability was found in a dependency of acme/widgets.\n\nPackage: left-trim (npm)\nAffected versions: below 2.0.4\nYour version: 2.0.1, in package-lock.json\n\nUpgrade to 2.0.4 or later. Review the alert on Codeforge. You are receiving this because you are an admin of this repository."),
    m(Notice, Held, "Tickwatch <alerts@tickwatch.example>", "Job failed: nightly-backup",
      "Your scheduled job nightly-backup did not finish.\n\nExpected at: 03:00\nLast run: exited with code 2 after 41 seconds\nOutput: rsync: connection refused (111)\n\nWe will email you again when it recovers. Pause alerts for this job."),
    m(Notice, Held, "Northbank Online <alerts@northbank.example>", "Your sign-in email was updated",
      "Hello Maya,\n\nThe email address you use to sign in to Northbank Online was changed on 15 Nov 2026 at 09:12.\n\nIf you made this change, there is nothing else to do. If you did not, call us straight away on the number on the back of your card.\n\nThis is a service message about your account."),
    m(Notice, Held, "Kiwi Market <messages@kiwimarket.example>", "Sam sent you a message about \"Oak desk\"",
      "Sam wrote:\n\n\"Hi, is the desk still available? I could pick it up on Saturday morning if that works for you. Would you take 80?\"\n\nReply in Kiwi Market. For your safety, keep conversations in the app. Notification settings."),
    m(Notice, Held, "Tasklane <tasks@tasklane.example>", "Jordan Blake assigned you: Write the onboarding email",
      "Jordan Blake assigned a task to you in Growth team.\n\nWrite the onboarding email\nDue: Friday 20 November\nNotes: First draft only, keep it under 150 words.\n\nOpen the task. You get these emails when someone assigns you work."),
    m(Notice, Held, "Drivebox <account@drivebox.example>", "Your Drivebox storage is 95% full",
      "Hi Maya,\n\nYou are using 1.9 GB of your 2 GB. When it is full, new files will stop syncing from your Mac and phone.\n\nDelete files you no longer need, or see storage options. Manage email preferences."),
    m(Notice, Held, "Inkwell <notify@inkwell.example>", "New comment on \"Notes on caching\"",
      "Arjun left a comment on your post Notes on caching:\n\n\"Great write-up. Did you measure the hit rate before and after the change?\"\n\nReply or moderate this comment. You get these because you write on Inkwell. Unsubscribe from comment emails."),
    // Newsletters, tuning half.
    m(Newsletter, Tune, "The Frontend Fortnight <hello@frontendfortnight.example>", "The Frontend Fortnight #112: container queries everywhere",
      "Issue 112\n\nThis fortnight: container queries land in every browser, a new way to animate height, and why your bundle is bigger than you think.\n\n1. Container queries are everywhere now\n2. Animating to auto\n3. Bundle size, explained\n\nJobs: Senior frontend engineer at Paintly.\n\nYou are receiving this because you subscribed. Unsubscribe or update your preferences."),
    m(Newsletter, Tune, "Oakline Outfitters <deals@oakline.example>", "30% off everything this weekend",
      "This weekend only: 30% off everything, in store and online.\n\nRain jackets, boots and the new merino range. Use code WEEKEND30 at checkout. Ends Sunday at midnight.\n\nShop now. Free returns.\n\nYou are receiving this because you signed up for Oakline emails. Unsubscribe."),
    m(Newsletter, Tune, "Lumen Cloud <news@lumencloud.example>", "Introducing Lumen Edge Functions",
      "Today we are launching Edge Functions: run your code in 40 regions, close to your users.\n\nCold starts under 5 ms, pay only for what you use, and deploy with one command.\n\nRead the launch post, or join the live demo on Thursday.\n\nThe Lumen Cloud team. Unsubscribe from product news."),
    m(Newsletter, Tune, "Fernwood Community <forum@fernwood.example>", "Your weekly Fernwood Community digest",
      "Top topics this week:\n\n- Firmware 4.2 is out: what changed (182 replies)\n- Best cases for the Fernwood One (64 replies)\n- Show us your desk setup (51 replies)\n\nNew members: 312\n\nYou are getting this digest because you have an account. Change how often you get it."),
    m(Newsletter, Tune, "Ana Ruiz from Field Notes <fieldnotes@letterbox.example>", "On slow mornings",
      "Field Notes, issue 37\n\nThis week I have been thinking about slow mornings: the hour before anyone needs anything from you, and how quickly it disappears once you open your email.\n\nHere is what I tried for a month, and what stuck.\n\nThanks for reading. Share this with a friend, or unsubscribe."),
    m(Newsletter, Tune, "DataHarbor <events@dataharbor.example>", "Live webinar: Scaling Postgres to a billion rows",
      "Join our engineers on 3 December at 17:00 CET for a live webinar on scaling Postgres.\n\nYou will learn: partitioning without downtime, choosing indexes, and when to shard.\n\nSave your seat, it is free. Cannot make it? Register and we will send you the recording.\n\nDataHarbor. Unsubscribe."),
    m(Newsletter, Tune, "Morning Ledger <briefing@morningledger.example>", "Tuesday briefing: rates hold, chip stocks slide",
      "Good morning. Here is what you need to know today.\n\nThe central bank held rates for a third month. Chip makers fell 4% after a weak forecast. And a new law on rental deposits starts in January.\n\nRead the full briefing. Forwarded this? Subscribe. Unsubscribe."),
    m(Newsletter, Tune, "Codeforge <changelog@codeforge.example>", "Codeforge monthly: merge queues for everyone",
      "New this month on Codeforge:\n\n- Merge queues on every plan\n- Search across all your repositories\n- A dark theme for the phone app\n- Bigger build machines, in beta\n\nRead the full notes. You are receiving this because you have a Codeforge account. Unsubscribe from product updates."),
    // Newsletters, held out.
    m(Newsletter, Held, "Bramble Bistro <hello@bramblebistro.example>", "Our autumn menu is here, plus 2-for-1 Tuesdays",
      "The autumn menu starts this week: squash risotto, slow-cooked lamb and a pear tart you will think about for days.\n\nEvery Tuesday in November, two mains for the price of one.\n\nBook a table. See you soon, the Bramble team. Unsubscribe."),
    m(Newsletter, Held, "Learnly <picks@learnly.example>", "Courses picked for you this week",
      "Based on what you have been learning, we think you will like:\n\n- Rust for web developers (4.8 stars)\n- Practical SQL (4.7 stars)\n- Designing data-heavy apps (4.9 stars)\n\nAll courses are 40% off until Sunday.\n\nLearnly. Unsubscribe from recommendations."),
    m(Newsletter, Held, "The Quiet Build <podcast@quietbuild.example>", "Episode 48: shipping alone",
      "New episode: Episode 48, Shipping alone.\n\nThis week Dev and Marta talk about running a product with no team: what to automate, what to drop, and how to take a holiday.\n\nListen wherever you get your podcasts. Thanks for subscribing. Unsubscribe."),
    m(Newsletter, Held, "Riverkeepers <team@riverkeepers.example>", "Help us clean up the Elm River",
      "Dear friend,\n\nLast year 400 volunteers pulled two tonnes of rubbish out of the Elm River. This spring we want to do it again, and we need your help.\n\nDonate today, or sign up to volunteer on 12 April.\n\nWith thanks, the Riverkeepers. Unsubscribe."),
    m(Newsletter, Held, "DevSummit <tickets@devsummit.example>", "Early bird tickets end Friday",
      "DevSummit 2027 is coming to Rotterdam on 14 and 15 May.\n\n60 talks, 12 workshops and the best hallway track in Europe. Early bird tickets are 35% off until Friday.\n\nGet your ticket. You are receiving this because you attended a past DevSummit. Unsubscribe."),
    m(Newsletter, Held, "Skyhop <deals@skyhop.example>", "Fares from 29 euros to Lisbon this winter",
      "Winter sun is closer than you think.\n\nLisbon from 29 euros, Seville from 34 euros, Malta from 39 euros. Book by Monday for travel until March.\n\nSee all deals. Prices are one way, including taxes. Unsubscribe from deals."),
    m(Newsletter, Held, "Papers This Week <digest@paperstw.example>", "Issue 61: indexes, consensus and fuzzing",
      "This week's reading list:\n\n1. Learned indexes revisited\n2. A simpler consensus protocol\n3. Energy use of on-device models\n4. Fuzzing parsers with grammars\n5. How developers read code\n\nA short summary of each inside. You signed up at paperstw.example. Unsubscribe."),
    m(Newsletter, Held, "Fernwood Community <forum@fernwood.example>", "Forum maintenance on Saturday",
      "Hello everyone,\n\nThe Fernwood Community forum will be down for maintenance on Saturday 21 November from 08:00 to 10:00 UTC while we move to new servers.\n\nNothing to do on your side. Thanks for your patience, the Fernwood team.\n\nYou are receiving this because you have a forum account. Unsubscribe from announcements."),
    // Written after the wording was fixed: notifications.
    m(Notice, Fresh, "Codeforge <notifications@codeforge.example>", "[acme/widgets] Merged: Add retry to the uploader (PR #412)",
      "Merged #412 into main.\n\nPriya Nair merged your pull request after 2 approvals. The branch maya/retry can be deleted.\n\nReply to this email directly, view it on Codeforge, or unsubscribe.\nYou are receiving this because you authored the thread."),
    m(Notice, Fresh, "Pipewright CI <builds@pipewright.example>", "[widgets] Build #2302 fixed on main",
      "Build #2302 passed. The build is fixed.\n\nCommit 9b1e004 by Maya Ross: Read retry count from config\nAll 214 tests passed in 3m 58s.\n\nView the build. You get these emails because you broke and fixed this branch. Manage notifications."),
    m(Notice, Fresh, "Trackly <notify@trackly.example>", "WID-91 moved to Done",
      "Jordan Blake changed the status of WID-91 Export fails for long names from In review to Done.\n\nYou are watching this issue because you reported it. Stop watching."),
    m(Notice, Fresh, "Papyr <share@papyr.example>", "Lena Park shared \"Hiring plan 2027\" with you",
      "Lena Park shared a document with you.\n\n\"Here is the draft we talked about. Could you add the design roles before Thursday?\"\n\nOpen Hiring plan 2027. You can edit."),
    m(Notice, Fresh, "Lumen Cloud <account@lumencloud.example>", "Your API key ends in 7 days",
      "Hi Maya,\n\nThe API key deploy-bot in project widgets-prod ends on 22 November. Anything that uses it will stop working then.\n\nMake a new key in the console and update your settings. Lumen Cloud. Email preferences."),
    m(Notice, Fresh, "Fernwood Community <forum@fernwood.example>", "Your answer was marked as the solution",
      "Good news: jlee marked your reply as the solution to Sync stops after the update.\n\nYou earned the Helpful badge. Visit the topic. You are getting this because you posted in this topic. Unsubscribe."),
    m(Notice, Fresh, "Tickwatch <alerts@tickwatch.example>", "DOWN: api.widgets.example is not responding",
      "Your monitor api.widgets.example is down.\n\nSince: 06:14 UTC\nCheck: HTTPS GET /health timed out after 30 s from 3 regions\n\nWe will tell you when it is back up. Acknowledge this alert."),
    m(Notice, Fresh, "Northbank Online <alerts@northbank.example>", "Your new PIN is in the post",
      "Hello Maya,\n\nThe new PIN you asked for on 12 November was posted today and should arrive within 5 working days. Your current PIN works until then.\n\nThis is a service message about your account."),
    // Written after the wording was fixed: newsletters.
    m(Newsletter, Fresh, "Paintly Blog <blog@paintly.example>", "7 ways to pick a better colour palette",
      "New on the Paintly blog: 7 ways to pick a better colour palette, from starting with one photo to testing in greyscale.\n\nAlso this month: a free palette pack and a talk from our design lead.\n\nRead the post. You subscribed to the Paintly blog. Unsubscribe."),
    m(Newsletter, Fresh, "Kiwi Market <hello@kiwimarket.example>", "Trending near you this week",
      "Popular near you this week: mid-century chairs, road bikes and standing desks.\n\nNew: sell faster with photo tips from top sellers.\n\nBrowse now. You get these because you have a Kiwi Market account. Unsubscribe from weekly picks."),
    m(Newsletter, Fresh, "Elm City Library <news@elmlibrary.example>", "November at the library: events and new arrivals",
      "This month at Elm City Library: a talk on local history on the 18th, coding club for teens every Wednesday, and 120 new books on the shelves.\n\nSee the full events calendar. Elm City Library news. Unsubscribe."),
    m(Newsletter, Fresh, "Codeforge <events@codeforge.example>", "Join us at Codeforge Universe 2027",
      "Codeforge Universe is back on 9 and 10 March in Lisbon and online.\n\nKeynotes, hands-on labs and the first look at what we are building next. Early tickets are 40% off this month.\n\nRegister now. You are receiving this because you have a Codeforge account. Unsubscribe from event emails."),
    m(Newsletter, Fresh, "Morning Ledger <briefing@morningledger.example>", "The weekend edition: the long reads",
      "Your weekend edition: the long reads.\n\nInside: how one town rebuilt its high street, the quiet return of night trains, and a guide to winter vegetables.\n\nRead it all. Forwarded this? Subscribe. Unsubscribe."),
    m(Newsletter, Fresh, "Oakline Outfitters <deals@oakline.example>", "Black Friday starts now",
      "Black Friday is here early: up to 50% off jackets, boots and bags.\n\nMembers get free shipping on everything until Monday.\n\nShop the sale. You are receiving this because you signed up for Oakline emails. Unsubscribe."),
    m(Newsletter, Fresh, "This Week in Ferrous <editor@ferrousweekly.example>", "This Week in Ferrous #77",
      "Issue 77\n\nNews: the new release is out with faster builds. Articles: writing a parser by hand, async without tears. Crate of the week: tidy-csv. Call for contributors: 6 open issues.\n\nYou are subscribed to This Week in Ferrous. Unsubscribe."),
    m(Newsletter, Fresh, "Lumen Cloud <news@lumencloud.example>", "Our new prices from 1 January",
      "From 1 January 2027 our prices change: compute goes down 10%, storage stays the same, and the free tier now includes 2 GB of bandwidth.\n\nSee the new prices and a calculator on our site. Thank you for building with Lumen Cloud.\n\nThe Lumen Cloud team. Unsubscribe from product news."),
    // Sales emails sent in bulk that read like a letter: marketing, so Newsletters.
    m(Pitch, Tune, "Brett Lyle <brett@fieldsales.example>", "Maya, a thought on Acme's sales hires",
      "Hi Maya,\n\nSaw that Acme is growing its sales team. We train new reps in two weeks instead of two months, with role-play sessions built from your own calls.\n\nWorth 15 minutes next week?\n\nBest,\nBrett\n\nNot the right person? Reply and let me know. Unsubscribe."),
    m(Pitch, Tune, "Brett Lyle <brett@fieldsales.example>", "Re: Maya, a thought on Acme's sales hires",
      "Hi Maya, raising this again in case my first note slipped by. I can send a two minute video instead of a call if that is easier.\n\nBrett\n\nUnsubscribe."),
    m(Pitch, Tune, "Nora Vance <nora@hirewell.example>", "Closing the loop",
      "Hi Maya,\n\nI have written a couple of times about helping Acme fill its open engineering roles and have not heard back, so I will stop here. If the timing changes, my calendar link is below.\n\nAll the best,\nNora\n\nUnsubscribe from these emails."),
    m(Pitch, Fresh, "Priya Sen <priya@spendlane.example>", "Maya, how long do Acme's expense reports take?",
      "Hi Maya,\n\nMost founders I talk to spend an afternoon a month chasing expense receipts. Spendlane collects them from your team's phones and files them for 39 a month.\n\nCan I show you in 10 minutes on Thursday?\n\nPriya\n\nOpt out of future emails."),
    m(Pitch, Fresh, "Owen Hart <owen@printpeak.example>", "Re: Acme's office printing",
      "Hi Maya, following up on my note last week. We just helped a company of Acme's size cut its printing costs by 40% with one managed printer plan. Could we find 15 minutes?\n\nOwen\n\nUnsubscribe."),
    m(Pitch, Fresh, "Tyler Grant <tyler@fleetsaver.example>", "Following up on your car leases",
      "Hi Maya,\n\nWhen do Acme's company car leases end? We usually get teams your size 10 to 15% off their next leases without changing models. Reply with a good time and I will send a quick comparison.\n\nTyler\n\nTo stop getting these emails, unsubscribe."),
    // Pitches sent like a letter, without list headers (15 November 2026), tuning half.
    d(Pitch, Tune, "Rahul Mehta <rahul@codecrest.example>", "Acme's roadmap for 2027",
      "Hi Maya,\n\nI came across Acme's widget app and was impressed by how fast it has grown. We are a team of 60 engineers in Pune who help product companies ship features faster, at a third of the cost of hiring locally.\n\nWould you be open to a quick call next week to see if we could help?\n\nBest regards,\nRahul Mehta\nGrowth lead, Codecrest"),
    d(Pitch, Tune, "Nathan Cross <nathan@harborlend.example>", "Funding without dilution",
      "Hello Maya,\n\nWe arrange revenue-based loans for software companies with steady subscription income. No equity, no board seat, and money in about two weeks.\n\nIf extra runway would help Acme next year, I can share terms that fit companies your size.\n\nNathan Cross\nHarbor Lending"),
    d(Pitch, Tune, "Sofia Lind <sofia@talentharbor.example>", "Senior engineers for Acme",
      "Hi Maya,\n\nI saw Acme is hiring. We place vetted senior engineers from Latin America in two weeks, in your time zone, for 40% less than local hires.\n\nShall I send over three profiles that match your stack?\n\nSofia Lind\nTalent Harbor"),
    d(Pitch, Tune, "Sofia Lind <sofia@talentharbor.example>", "Re: Senior engineers for Acme",
      "Hi Maya, just floating this back up. Happy to send the profiles over with no commitment.\n\nSofia"),
    d(Pitch, Tune, "Chloe Martin <chloe@buzzlane.example>", "Getting Acme into the news",
      "Hi Maya,\n\nWe are a small PR agency for consumer apps. Last quarter we placed one client in 40 news stories in a week.\n\nIf Acme has a launch coming up, I would love to pitch it to the reporters we work with. Free to chat on Thursday?\n\nChloe\nBuzzlane PR"),
    d(Pitch, Tune, "Victor Osei <victor@creditwise.example>", "Tax credits Acme may be missing",
      "Hi Maya,\n\nBuilding software like Acme's is often eligible for R&D tax credits, and most companies claim less than half of what they could. We work on contingency, so there is no fee unless you get money back.\n\nWho handles this at Acme?\n\nVictor Osei\nCreditwise"),
    d(Pitch, Tune, "Hannah Cole <hannah@leadspring.example>", "One last try",
      "Maya, I have written a few times without luck, so this is my last note. Our sponsored newsletter slots reach 5,000 product managers a month, and there is one left for December. Shall I hold it for Acme?\n\nHannah"),
    d(Pitch, Tune, "Marcus Webb <marcus@outboundly.example>", "More demos for Acme",
      "Maya,\n\nWe run cold email campaigns end to end for software companies: we find the leads, write the emails and book the calls. You only pay for meetings that happen.\n\nOpen to seeing how it would work for Acme?\n\nMarcus"),
    d(Pitch, Tune, "Grace Kim <grace@summitworks.example>", "Acme at Product Summit Toronto",
      "Hi Maya,\n\nWe are putting together the startup showcase for Product Summit Toronto on 4 March and Acme would be a great fit. Exhibitor spots start at 1,500 and include two passes.\n\nShall I hold a spot for you?\n\nGrace Kim\nPartnerships, Summit Works"),
    d(Pitch, Tune, "Leo Grant <leo@clipcast.example>", "Faster product videos",
      "Hi Maya,\n\nClipcast turns a rough screen recording into a polished product video with captions and music in one click. Teams like yours use it for onboarding and release notes.\n\nCan I give you a free month to try it?\n\nLeo"),
    r(Pitch, Tune, "Pictora Partnerships <partners@pictora-desk.example>", "Ticket 48211: a partnership idea for Acme",
      "Ticket 48211\n\nHello Maya,\n\nWe run a review site for creative software read by two million designers each month. We would like to write about Acme's app and give your users a discount code, in exchange for a link from your site.\n\nJust reply to this ticket if you are interested.\n\nPictora Partnerships"),
    // Pitches, held out.
    d(Pitch, Held, "Arjun Rao <arjun@devbridge-labs.example>", "Engineering help for Acme",
      "Dear Maya,\n\nDevbridge Labs builds web and mobile apps for product companies, with teams in Krakow and Lisbon. We can add three engineers to your team within a month.\n\nWould a short introduction call next week suit you?\n\nRegards,\nArjun Rao"),
    d(Pitch, Held, "Peter Marsh <peter@ridgelinefunds.example>", "Re: Growth capital for Acme",
      "Hi Maya,\n\nFollowing up on my note from last week. Our fund still has room for one more software company this quarter, and we can move from first call to term sheet in three weeks.\n\nIs this worth 15 minutes?\n\nPeter Marsh\nRidgeline Funds"),
    d(Pitch, Held, "Jay Patel <jay@viralnest.example>", "Feature Acme on our design pages",
      "Hey Maya,\n\nWe run design inspiration pages with 2 million followers. A sponsored post about Acme's app costs 300 and goes out this week.\n\nWant the media kit?\n\nJay"),
    d(Pitch, Held, "Ella Brooks <ella@ledgerline.example>", "Bookkeeping for Acme",
      "Hello Maya,\n\nWe do monthly bookkeeping and year-end accounts for small software companies, for a flat fee. Most clients save a day a month.\n\nWould you like a quote?\n\nElla Brooks\nLedgerline"),
    d(Pitch, Held, "Nadia Flores <nadia@connectpoint.example>", "Lower phone bills for Acme",
      "Hi Maya,\n\nConnectPoint resells business phone and internet plans and usually cuts our clients' bills by a quarter. A free review of your current plan takes ten minutes.\n\nWho should I speak to at Acme?\n\nNadia Flores"),
    d(Pitch, Held, "Omar Haddad <omar@hireloop.example>", "Designers and developers on demand",
      "Hello Maya,\n\nHireloop finds vetted designers and developers for short projects, ready to start within days. Clients pay only for the hours worked.\n\nCould you point me to whoever handles hiring at Acme?\n\nOmar"),
    d(Pitch, Held, "Ruby Shaw <ruby@appascend.example>", "Re: Acme's app store ranking",
      "Hi Maya, one more note and then I will leave you be. We help apps climb the store rankings with better screenshots, keywords and review prompts. If that is ever on your list, my calendar is open.\n\nRuby\nAppAscend"),
    d(Pitch, Held, "Ben Novak <ben@trustmark-advisory.example>", "Getting Acme ready for the new data rules",
      "Hi Maya,\n\nNew data protection rules start in January, and most small software companies are not ready. Trustmark updates your policies and paperwork in two weeks for a fixed fee.\n\nWould a short call help?\n\nBen Novak"),
    d(Pitch, Held, "Iris Moreau <iris@luxeforum.example>", "Re: Founders' dinner in Paris",
      "Hello Maya,\n\nA reminder about our founders' dinner in Paris on 20 March. A few sponsored seats are left, each at a table with our guest speakers.\n\nShall I keep one for Acme?\n\nIris"),
    d(Pitch, Held, "Tom Reyes <tom@coverwise.example>", "Office insurance for Acme",
      "Hi Maya,\n\nWe compare business insurance for small tech companies and usually find a better price within a day. When does Acme's current policy end?\n\nTom Reyes\nCoverwise"),
    r(Pitch, Held, "Brightsoft <noreply@brightsoft-partners.example>", "Acme: an invitation to partner",
      "Hi Maya,\n\nWe are inviting a few software companies to join the Brightsoft partner program: list your app in our marketplace, reach 50,000 businesses and earn a share of every sale.\n\nApply in two minutes. Spots for this quarter are limited.\n\nThe Brightsoft Partnerships team"),
    m(Pitch, Held, "Kara Diaz <kara@growthpilot.example>", "Demo bookings for Acme",
      "Hi Maya,\n\nWe help software teams double their demo bookings in 90 days with ads and landing pages. If marketing is not your area, who would be the best person to talk to?\n\nKara\n\nUnsubscribe"),
    m(Pitch, Held, "Liam Ford <liam@stackhire.example>", "Engineers for Acme, ready in a week",
      "Hi Maya,\n\nWe have five senior backend engineers available from next week, all with Rust experience. Want me to send their profiles?\n\nLiam\n\nIf you would rather not hear from me, unsubscribe here."),
    // People writing for the first time: Important, never Newsletters. Tuning half.
    d(Person, Tune, "Ines Carvalho <ines.carvalho@mailbox.example>", "Export stops at 99%",
      "Hi,\n\nI bought the widget app last week and every time I export a project it stops at 99% and nothing happens. I am on a Mac with the latest version. Is there something I can try? I have a client deadline on Friday.\n\nThanks,\nInes"),
    d(Person, Tune, "Daniel Okoro <daniel.okoro@postbox.example>", "Lena said I should write to you",
      "Hi Maya,\n\nLena Park gave me your email. I am moving to Porto in January and she said you know the city well. Would you have 20 minutes for a coffee or a call sometime next week? I would really appreciate any tips on neighbourhoods.\n\nThanks so much,\nDaniel"),
    d(Person, Tune, "Priyanka Rao <priyanka@voxscribe.example>", "How is the transcription API working for you?",
      "Hi Maya,\n\nI am on the product team at Voxscribe. Your account has been on the paid plan for two months, and I would love to hear what works and what doesn't. Would you have 20 minutes this week, or could you reply with a line or two?\n\nPriyanka"),
    d(Person, Tune, "Tariq Hussain <tariq.h@inbox.example>", "Application: junior designer",
      "Hi Maya,\n\nI saw the junior designer opening on Acme's site and attached my portfolio and CV. I have two years of experience in product design for mobile apps, and I use your widget app myself every day.\n\nI would love to hear from you.\n\nBest,\nTariq Hussain"),
    d(Person, Tune, "Alice Wren <alice.wren@dailytech.example>", "Interview request for a story on small app makers",
      "Hi Maya,\n\nI am a reporter at Daily Tech writing a story on small companies that build creative tools, and Acme came up several times. Would you have 15 minutes for a phone interview this week? My deadline is Thursday.\n\nAlice Wren"),
    d(Person, Tune, "Rui Santos <rui.santos@postbox.example>", "Water leak from your flat?",
      "Hi Maya, I live in the flat below yours (2B). There is water coming through my bathroom ceiling since this morning. Could you check if something is leaking on your side? I am home all day.\n\nRui"),
    // People writing for the first time, held out.
    d(Person, Held, "Mei Chen <mei.chen@mailbox.example>", "Refund for a double charge",
      "Hello,\n\nI was charged twice for my yearly plan on 3 November. Could you refund one of the payments? My account email is this one.\n\nThank you,\nMei Chen"),
    d(Person, Held, "Lucas Moreau <lucas@studiomoreau.example>", "Speaking at our design meetup?",
      "Hi Maya,\n\nI run a small design meetup in Lisbon, about 80 people every month. We loved your talk on colour at last year's conference. Would you like to speak at our February session? It is unpaid, but we will cover your travel.\n\nLucas"),
    d(Person, Held, "Hana Sato <hana.sato@uninorth.example>", "Question about your post on caching",
      "Dear Maya,\n\nI am a PhD student at Uni North and read your post on caching with great interest. Could I ask how you measured the hit rate? I am trying to reproduce it for my thesis.\n\nBest wishes,\nHana Sato"),
    d(Person, Held, "Olga Petrova <olga.p@inbox.example>", "Found your wallet",
      "Hi, I found a wallet on the 14 tram with your business card in it. I can drop it at your office or meet you somewhere. Let me know what works.\n\nOlga"),
    d(Person, Held, "Sanjay Iyer <sanjay@brightline.example>", "Pricing for 200 seats",
      "Hi Maya,\n\nWe are looking at the widget app for our design team of about 200 people. Do you offer volume pricing, and can we pay by invoice? We would like to decide before the end of the month.\n\nSanjay Iyer\nHead of Design, Brightline"),
    d(Person, Held, "Grace Liu <grace.liu@mailbox.example>", "Your old flat on Elm Street",
      "Hi Maya,\n\nI moved into your old flat on Elm Street last month and a few letters for you still arrive here, one from the tax office. Should I forward them somewhere?\n\nGrace"),
    // People the user has written to: Important. Tuning half.
    d(Contact, Tune, "Marco Bianchi <marco@bianchibuild.example>", "Re: Quote for the bathroom",
      "Hi Maya, I can start on the 9th if that still suits you. I will bring the tile samples on Monday so you can choose.\n\nMarco"),
    d(Contact, Tune, "Helen Ward <helen@wardaccounts.example>", "Receipts for the quarter",
      "Hi Maya,\n\nI am closing the quarter this week. Could you upload the September receipts to the shared folder by Thursday?\n\nThanks,\nHelen"),
    d(Contact, Tune, "Lena Park <lena@acme.example>", "Design crit notes",
      "Hi Maya, here are my notes from today. The main thing: the export dialog needs a progress bar. Can we talk about it tomorrow?\n\nLena"),
    // People the user has written to, held out.
    d(Contact, Held, "Carlos Vega <carlos@vegarenovations.example>", "Re: Quote request: kitchen tiles",
      "Hi Maya, just following up on the quote I sent last week. Happy to answer any questions, and I can still start on the 2nd if you want to go ahead.\n\nCarlos"),
    d(Contact, Held, "Nina Hart <nina.hart@northbank.example>", "Re: Mortgage renewal",
      "Hi Maya,\n\nThanks for your note. I have attached the two renewal options we talked about on the phone. Once you pick one, I will send the papers to sign.\n\nNina"),
    d(Contact, Held, "Priya Nair <priya@acme.example>", "Can you review the uploader PR today?",
      "Hey Maya, the uploader fix is ready. Could you take a look before 4? I want it in tonight's release.\n\nPriya"),
    // Notices and promotions without list headers. Tuning half.
    d(Notice, Tune, "Keyring <hello@keyring.example>", "Keyring: new device signed in",
      "A new device signed in to your Keyring vault: Safari on Mac, Porto, 6 November at 17:13.\n\nIf this was you, there is nothing to do. If not, lock your vault and change your master password."),
    d(Notice, Tune, "Northbank <service@northbank.example>", "Please confirm your phone number",
      "Hi Maya,\n\nWe could not reach you on the phone number we have on file. Sign in to online banking to check your contact details, so we can reach you about your account."),
    r(Notice, Tune, "Lumen Cloud <noreply@lumencloud.example>", "A new login to your Lumen Cloud account",
      "Hello Maya,\n\nSomeone signed in to your Lumen Cloud account from Chrome on Mac in Porto.\n\nIf this was you, no need to do anything. If not, reset your password."),
    r(Notice, Tune, "Uptimer <noreply@uptimer.example>", "Alert: api.acme.example returned 502",
      "Your monitor api.acme.example returned HTTP 502 Bad Gateway from 3 of 4 regions at 03:14 UTC.\n\nWe will email you when it recovers."),
    d(Newsletter, Tune, "Lark & Linen <rewards@larklinen.example>", "Your birthday treat is here",
      "Hello Maya,\n\nHappy birthday from all of us! Enjoy 20% off anything in store or online until the end of the month.\n\nShop now."),
    r(Newsletter, Tune, "Homeway <no-reply@homeway.example>", "Give 50, get 50",
      "Know someone buying a home? Send them your link: they get 50 off their first service with Homeway, and so do you.\n\nShare your link today."),
    // Notices and promotions without list headers, held out.
    d(Notice, Held, "Fernbank Invest <info@fernbank.example>", "Your quarterly report is ready",
      "Hi Maya,\n\nYour report for July to September is ready to read in the app.\n\nFernbank Invest"),
    d(Notice, Held, "Payfast <alerts@payfast.example>", "Payment sent to Jo Lennon",
      "Hi Maya,\n\nYour payment of 240.00 to Jo Lennon was received.\n\nReference: PF-77Q1\n\nThis is an automatic message."),
    r(Notice, Held, "Payline <notifications@payline.example>", "A chargeback was decided in your favour",
      "Good news: the card company ruled for you on the chargeback for order 5531. The 42.00 and the fee come back to your balance in 3 days.\n\nSee the details in your dashboard."),
    r(Notice, Held, "Coinmint <no-reply@coinmint.example>", "ETH crossed your alert price",
      "Ether is now above 4,000, the price you set for this alert.\n\nManage your alerts in the app."),
    d(Newsletter, Held, "Fontwell <hello@fontwell.example>", "The Fontwell lifetime licence is back",
      "Hi Maya,\n\nYou bought a Fontwell licence a while ago. This week only, the lifetime licence is 30% off: every future font update, paid once.\n\nGet the lifetime licence."),
    // Added after the first check on a real mailbox found these two kinds of mistake: a
    // company answering the user's own request, and a service confirming the user's own setup.
    d(Person, Tune, "Brightwall Customer Care <care@brightwall.example>", "Your claim 01928374",
      "Dear Maya Ross,\n\nWe are sorry the paint you bought peeled within a month. As you asked, we will replace it: pick up two gallons of any Brightwall colour at your local store with this claim number.\n\nClaim number: 01928374\n\nBrightwall Customer Care"),
    r(Notice, Tune, "Riverside Gas <donotreply@riversidegas.example>", "Your automatic payments are set up",
      "Hi Maya Ross,\n\nAutomatic payments are now on for your Riverside Gas account. We will take each bill from your bank account on its due date, starting in December.\n\nYou can turn this off in My Account at any time."),
    d(Person, Held, "Northline Customer Relations <relations@northline.example>", "Your feedback: case 554120",
      "Dear Maya Ross,\n\nThank you for writing to us about your delayed flight on 2 November. We are sorry for the trouble. As a gesture of goodwill we have added 5,000 points to your account, and you can claim back 60 for your meals with the form below.\n\nKind regards,\nNorthline Customer Relations"),
    r(Notice, Held, "Cityline Water <noreply@citylinewater.example>", "Your new water account",
      "Dear Maya Ross,\n\nYour Cityline Water account is open. Your first bill comes in about a month, and you can see your usage any time in My Account."),
    // Added after the held-out score: a would-be customer asking to buy reads like a pitch.
    d(Person, Tune, "Mateo Ruiz <mateo@harborprint.example>", "Licences for our studio",
      "Hi Maya,\n\nWe are a print studio with 35 designers and would like to buy the widget app for everyone. Is there a team plan, and can you send a quote we can pay by bank transfer?\n\nMateo Ruiz\nOperations, Harbor Print"),
    r(Newsletter, Held, "Stockleaf <notifications@stockleaf.example>", "New on Stockleaf: crypto baskets",
      "You can now buy a ready-made basket of the ten biggest coins in one tap, from as little as 5.\n\nTry it in the app."),
    // Written once the pitch questions were fixed (15 November 2026), before scoring them.
    d(Pitch, Late, "Noah Bell <noah@pixelforge-studio.example>", "Design support for Acme",
      "Hi Maya,\n\nWe are a design studio that works with software teams on landing pages, onboarding and app store screenshots. We have helped 40 startups lift sign-ups by a third.\n\nCould I send you a few examples of our work?\n\nNoah Bell"),
    d(Pitch, Late, "Lara Quinn <lara@seedbridge.example>", "Acme's next round",
      "Hi Maya,\n\nOur fund backs design tool companies at seed and Series A, and Acme keeps coming up in our research. Would you be open to a 20 minute intro call to see if there is a fit?\n\nLara Quinn\nPartner, Seedbridge"),
    d(Pitch, Late, "Kevin Duarte <kevin@cloudsavvy.example>", "Re: your hosting bill",
      "Hi Maya, checking in once more. We audit cloud bills for free and usually find 30% in savings within a week. Shall I run one for Acme?\n\nKevin"),
    d(Pitch, Late, "Priya Kapoor <priya@nearshorely.example>", "QA testers for Acme's releases",
      "Hello Maya,\n\nShipping fast is hard when testing falls behind. Our QA team in Bogota can test every release of your app within 24 hours, from 15 an hour.\n\nWould you like a free test of your next release?\n\nPriya Kapoor"),
    d(Pitch, Late, "Sam Ortiz <sam@podreach.example>", "Guest spot on our podcast?",
      "Hi Maya,\n\nWe produce sponsored founder interviews for a podcast with 30,000 listeners. Spots start at 900 and include a clip for your socials.\n\nInterested in a slot this month?\n\nSam"),
    d(Pitch, Late, "Erin Walsh <erin@b2bdata.example>", "Contact list for design leads",
      "Hi Maya,\n\nWould you be interested in a verified list of 12,000 design leaders in North America, with emails and phone numbers, for your next campaign?\n\nI can send a free sample of 50 contacts.\n\nErin"),
    d(Pitch, Late, "Dmitri Volkov <dmitri@legalease.example>", "Bumping this up",
      "Hi Maya, bumping this up in case it slipped past. Our trademark team can register Acme in 30 countries for a fixed fee. Worth a quick call?\n\nDmitri"),
    d(Pitch, Late, "Hugo Lambert <hugo@talentrise.example>", "VP of Engineering role",
      "Hi Maya,\n\nI am a recruiter working with a fast-growing design platform in Berlin. They are looking for a VP of Engineering, and your background stood out. Would you be open to a confidential chat?\n\nHugo Lambert\nTalentRise"),
    r(Pitch, Late, "Listingboost <noreply@listingboost.example>", "Feature Acme in our app directory",
      "Hello Maya,\n\nWe would like to feature Acme in the Listingboost app directory, seen by 200,000 buyers a month. Featured listings start at 49 a month, and the first month is free.\n\nClaim your listing today.\n\nThe Listingboost team"),
    d(Person, Late, "Joanna Field <joanna@northfield-school.example>", "Licences for our design class",
      "Hello,\n\nI teach design at Northfield School and would like to use the widget app with 25 students next term. Do you have an education discount, and how would we buy the licences?\n\nThanks,\nJoanna Field"),
    d(Person, Late, "Ahmed Saleh <ahmed@crescentbank.example>", "Security questions before we buy",
      "Hi Maya,\n\nOur team would like to roll out the widget app to 60 people. Before we can buy, our security team needs answers to a short questionnaire about where data is stored. Could you fill it in by next Friday?\n\nAhmed Saleh\nIT Procurement, Crescent Bank"),
    d(Person, Late, "Jonas Berg <jonas.berg@mailbox.example>", "Feature request: export to PDF",
      "Hi Maya,\n\nI use the widget app every day for client work. Would it be possible to export straight to PDF? Right now I go through two other apps. Happy to test a beta.\n\nJonas"),
    d(Person, Late, "Fatima Noor <fatima@designweek-porto.example>", "Invitation to speak at Design Week Porto",
      "Dear Maya,\n\nOn behalf of Design Week Porto, I would like to invite you to give a 30 minute talk on 12 May. We cover travel and two nights in a hotel, and we would love a talk on building tools for designers.\n\nCould you let me know by the end of the month?\n\nFatima Noor\nProgramme lead"),
    d(Person, Late, "Tomas Silva <tomas.silva@postbox.example>", "Your bike on the stairs",
      "Hi Maya, I am your new neighbour on the third floor. Your bike is chained to the stair rail and the landlord says it has to move by Sunday. Can I help you carry it down?\n\nTomas"),
    d(Person, Late, "Elise Varga <elise@brightpath.example>", "Freelance project?",
      "Hi Maya,\n\nA friend recommended you. We need a designer for a 6 week app redesign starting in January, paid by the day. Are you available, and what is your rate?\n\nElise Varga\nBrightpath"),
    d(Contact, Late, "Ingrid Holm <ingrid@holmlaw.example>", "Re: Lease review",
      "Hi Maya, I have gone through the lease. Two clauses worry me, see my notes attached. Can we talk on Tuesday?\n\nIngrid"),
    d(Contact, Late, "Theo Brandt <theo@signforge.example>", "Re: Pricing for 10 seats",
      "Hi Maya, thanks for your interest! As promised, here is our pricing for 10 seats with the startup discount. I would be happy to walk you through a demo this week.\n\nTheo"),
    r(Notice, Late, "Lumen Cloud <noreply@lumencloud.example>", "Your free trial ends in 3 days",
      "Hi Maya,\n\nYour Lumen Cloud trial ends on 18 November. Add a payment method to keep your projects running. Projects without one are paused, not deleted."),
    d(Notice, Late, "Northbank <alerts@northbank.example>", "Large purchase on your card",
      "Hi Maya, a purchase of 1,240.00 at ELECTRO WORLD was made on your card ending 4417. If this was not you, call us now."),
];

fn ai() -> Ai {
    let dir = PathBuf::from(std::env::var("MOSS_AI_DIR").expect("MOSS_AI_DIR"));
    Ai::new(Parts {
        dir,
        machine: Machine::detect(),
        catalog: Catalog::pinned(),
        launcher: Arc::new(Llama),
        events: Arc::new(|_| {}),
        free_space,
        runtime: std::env::var_os("MOSS_LLAMA_SERVER").map(PathBuf::from),
        conditions: moss_ai::power::conditions,
        recheck: std::time::Duration::from_secs(30),
        apple: None,
    })
}

const TABS: [&str; 5] = ["newsletters", "other", "important", "receipts", "calendar"];

/// Rows: what it is. Columns: the tab Moss puts it in.
#[derive(Default)]
struct Matrix(BTreeMap<(String, String), usize>);

impl Matrix {
    fn add(&mut self, kind: &str, tab: &str) {
        *self.0.entry((kind.into(), tab.into())).or_default() += 1;
    }

    fn count(&self, kind: &str, tab: &str) -> usize {
        self.0.get(&(kind.to_string(), tab.to_string())).copied().unwrap_or(0)
    }

    fn print(&self, title: &str) {
        eprintln!("\n{title}\n{:<11}{}", "", TABS.map(|t| format!("{t:>12}")).join(""));
        let kinds: std::collections::BTreeSet<&String> = self.0.keys().map(|(k, _)| k).collect();
        for kind in kinds {
            let row = TABS.map(|t| format!("{:>12}", self.count(kind, t)));
            eprintln!("{kind:<11}{}", row.join(""));
        }
    }
}

/// Where each kind belongs: pitches and newsletters in Newsletters, notices anywhere else,
/// people in Important.
fn right(kind: Kind, tab: &str) -> bool {
    match kind {
        Newsletter | Pitch => tab == "newsletters",
        Notice => tab != "newsletters",
        Person | Contact => tab == "important",
    }
}

/// Which version of Moss sorts.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Moss {
    /// Before 15 November 2026: only list mail and Important mail are asked, the split question.
    Before,
    /// Now: a stranger's mail in Important, and any mail in Other, is also asked whether it is
    /// a sales pitch.
    Now,
}

/// Where Moss puts an email, the way the app asks and decides (`AiSorter` in src-tauri,
/// `sort::decide` in moss-engine). `known`: the user has written to the sender, or they are
/// otherwise known (`classify::sender_status`).
async fn sorted(ai: &Ai, moss: Moss, via: Via, known: bool, email: &str) -> String {
    let sure = |p: f64| p >= prompt::SURE;
    // A pitch: sure it is one, and not sure the sender needs something of the user's.
    let pitch = || async {
        let (pitch, wants) = ai.pitch(email).await.unwrap();
        sure(pitch) && !sure(wants)
    };
    match via {
        List => {
            let (best, likely) = ai.list_split(email).await.unwrap();
            if best == "important" && !sure(likely) { "other".into() } else { best }
        }
        Direct => {
            if moss == Moss::Now && !known && pitch().await {
                return "newsletters".into();
            }
            let (best, likely) = ai.split(email).await.unwrap();
            if sure(likely) { best } else { "important".into() }
        }
        Robot if moss == Moss::Now && pitch().await => "newsletters".into(),
        Robot => "other".into(),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the downloaded decision model"]
async fn mail_is_sorted_by_what_it_is() {
    let ai = ai();
    assert!(ai.decider_ready(), "{:?}", ai.status());
    let mut matrices: BTreeMap<String, Matrix> = BTreeMap::new();
    let mut wrong: BTreeMap<String, usize> = BTreeMap::new();
    for mail in MAIL {
        let email = prompt::email(mail.from, mail.subject, &[], mail.body);
        let mut line = String::new();
        for moss in [Moss::Before, Moss::Now] {
            let tab = sorted(&ai, moss, mail.via, mail.kind == Contact, &email).await;
            let ok = right(mail.kind, &tab);
            line.push_str(&format!("{tab:<12}{:<6}", if ok { "" } else { "WRONG" }));
            matrices.entry(format!("{moss:?} {:?}", mail.half)).or_default().add(&format!("{:?}", mail.kind), &tab);
            *wrong.entry(format!("{moss:?} {:?}", mail.half)).or_default() += usize::from(!ok);
        }
        eprintln!("{:<6}{:<11}{:<7}{line}{}", format!("{:?}", mail.half), format!("{:?}", mail.kind), format!("{:?}", mail.via), mail.subject);
    }
    for (name, matrix) in &matrices {
        matrix.print(name);
        eprintln!("{name}: {} wrong", wrong[name]);
    }
    // The guard rails. No one the user writes with goes to Newsletters. People writing for the
    // first time and notices that go there now and didn't before: on 15 November 2026 the model
    // took two for pitches, an offer of paid work and a trial-ending notice (both `Late`).
    let strays = |m: &Matrix| m.count("Person", "newsletters") + m.count("Notice", "newsletters");
    let mut new_strays = 0;
    for half in ["Tune", "Held", "Fresh", "Late"] {
        let (before, now) = (&matrices[&format!("Before {half}")], &matrices[&format!("Now {half}")]);
        assert_eq!(now.count("Contact", "newsletters"), 0, "{half}");
        new_strays += strays(now).saturating_sub(strays(before));
        assert!(wrong[&format!("Now {half}")] <= wrong[&format!("Before {half}")], "{half}: worse than before");
    }
    assert!(new_strays <= 2, "{new_strays} people or notices newly in Newsletters");
}

/// A list of emails to sort, from a file: what each is (`kind`, any label), where the rules
/// put it (`split`: "important", "other" or "newsletters"), and whether the user knows the
/// sender (`known`).
#[derive(serde::Deserialize)]
struct Listed {
    #[serde(default = "mail")]
    kind: String,
    #[serde(default = "newsletters")]
    split: String,
    #[serde(default)]
    known: bool,
    from: String,
    subject: String,
    #[serde(default)]
    attachments: Vec<String>,
    body: String,
}

fn mail() -> String {
    "mail".into()
}

fn newsletters() -> String {
    "newsletters".into()
}

/// `MOSS_LIST_MAIL=<file.json>` sorts a JSON list of [`Listed`] emails (say, made from a copy
/// of your own mailbox, kept out of the repo) and prints only how many of each kind land in
/// each tab, before and now.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the decision model and a file of mail"]
async fn mail_from_a_file_is_counted_by_tab() {
    let ai = ai();
    let file = std::env::var("MOSS_LIST_MAIL").expect("MOSS_LIST_MAIL");
    let mail: Vec<Listed> = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let mut matrices: BTreeMap<String, Matrix> = BTreeMap::new();
    let started = std::time::Instant::now();
    for m in &mail {
        let via = match m.split.as_str() {
            "important" => Direct,
            "other" => Robot,
            _ => List,
        };
        let email = prompt::email(&m.from, &m.subject, &m.attachments, &m.body);
        for moss in [Moss::Before, Moss::Now] {
            let tab = sorted(&ai, moss, via, m.known, &email).await;
            matrices.entry(format!("{moss:?}")).or_default().add(&m.kind, &tab);
        }
    }
    eprintln!("{} emails in {:?}", mail.len(), started.elapsed());
    for (name, matrix) in &matrices {
        matrix.print(name);
    }
}
