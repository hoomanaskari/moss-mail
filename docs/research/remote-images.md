# Remote images: when Moss shows them

Decision record · 2 November 2026 · Status: **built**.

## The problem

Moss hid every remote image until you pressed "Show images", and forgot that choice as soon as you left the email. You had to say yes again and again, even for people you write to every week.

## What other mail apps do

| App | Default | Who is trusted | Privacy |
|---|---|---|---|
| Apple Mail | Loads everything, early, through Apple's relay | None | Hides your IP and when you opened it |
| Gmail | Shows images; asks for suspicious senders | Gmail's own judgement | Google image proxy |
| Outlook | Blocks, except Safe Senders | Safe Senders list (address or domain), contacts on Mac | None |
| Fastmail | Choice: always / contacts / always ask | Your contacts | Server proxy |
| Thunderbird | Blocks with a bar | What you allowed before | Per sender or domain |
| Proton | Shows images | None | Removes known trackers, proxy |
| HEY | Proxied | None | Removes pixels, names the tracker |
| Spark | Shows images | None | Blocks 1×1 pixels |
| Mimestream | Shows images | None | Blocks 75+ tracker services |
| Tuta | Blocks | "Always trust sender" | Show once / always trust / always block |

Sources: Apple, Google, Microsoft, Fastmail, Mozilla, Proton, HEY, Spark, Mimestream and Tuta help pages; Englehardt et al., *I never signed up for this!*, PETS 2018.

**No shipped mail app uses a learned model for this.** They all use rules: trusted senders, remembered choices, and lists of known trackers. Research classifiers exist, but they disagree a lot and are easy to evade.

## What Moss does

1. **Shows images by itself** when the sender is familiar and the account's mail server confirmed the email really comes from that sender's domain (`Authentication-Results`, see `crates/moss-store/src/authres.rs`). Moss reads that for Gmail, Microsoft, iCloud and Fastmail accounts. For other accounts it always asks first, and when the server didn't confirm the sender it says "Images are off because Moss couldn't confirm who sent this." Familiar means:
   - you have sent mail to that address before, or
   - you have sent mail to someone at the same company domain (not public ones such as gmail.com), or
   - you said "show images from" this sender before, or
   - you accepted (or unblocked) this sender, or everyone at their domain (⇧A): saying they're welcome covers their images too, so Moss doesn't ask twice.
2. **Asks** for anyone else, with two choices:
   - **Always for this sender** (⇧I) remembers the sender in that account. It isn't offered when the server didn't confirm the sender.
   - **Just this once** (⌥I) shows them for this email until Moss closes. It works for any email.

   The button doesn't repeat the sender's name: names like "The Slow Garden" or "Amazon.ca" have no reliable short form, and the name is shown just above.
3. **Removes images matching possible-tracker rules**, even when images are shown:
   - tiny (≤ 2 px) or hidden images, and
   - images whose web address looks like open tracking (`/open`, `/track`, `/o.gif`…) or contains your email address.

   These heuristics can miss trackers and remove ordinary images. The notice counts possible trackers, not proven tracking attempts. Remote image requests go directly to the host; Moss does not proxy them or hide your IP address.
4. **Uses a no-referrer policy** for image loads. This suppresses the referring URL; it does not hide your IP address or the timing of requests.

The familiar-sender signals come from mail already on your Mac, so deciding is instant and works offline.

## Not now

- **The decision model.** Rules are instant, predictable and match what every other app does. A 1 GB model adds nothing here.
- **Fetching images through Moss's Rust side**, with a cache, no cookies, and prefetching for trusted senders at sync time. Without its own server Moss can't hide your IP address; this would hide when you opened an email. It's the next privacy step.
- **A "never show images from" option**, and domain choices in Settings.
- **Bundling a tracker list.** `simplify-trackers` is BSD-3, so it's usable; EasyPrivacy (GPL / CC BY-SA), DuckDuckGo Tracker Radar (non-commercial) and Disconnect (non-commercial) are not.


## Optional strict image privacy

Settings → Appearance → Remote images offers **Ask for every message**. Familiar senders remains the default. Strict mode ignores inferred familiarity, remembered sender allowances and the own-message exemption when rendering both ordinary mail and forwarded previews. Only an explicit **Just this once** choice permits remote content for that message until Moss closes. Existing per-message choices remain valid when changing the policy. Already completed image requests cannot be undone by switching modes.

The sender-wide button is hidden in strict mode, and its keyboard shortcut cannot write an allowance. The preference is stored separately on this device; changing theme or opening a dev scene cannot silently overwrite it. Missing storage uses the established familiar default. Unreadable saved values or unavailable reads fall back to asking. If a choice cannot be saved, Settings reports that it applies only until Moss closes. A subsequent launch may still read the previously saved choice.

Familiar loading relies on the account's mail server to say the sender is real: an address alone is easy to forge, so it never loads images by itself. The familiar signals are convenience rules on top of that check. Disabling remote images does not disable local CID/data images. Tracker removal remains heuristic in both modes. `pnpm test:native-email` checks the resulting documents in WebKit; see [native-email-security.md](../native-email-security.md).
