# Native email security checks

```bash
pnpm test:native-email
```

Runs on a Mac. It is part of `pnpm check`.

## What it does

1. Renders made-up hostile emails through the real `EmailBody` component, and saves each frame's document and sandbox setting.
2. Builds a small Swift probe and loads those documents in WebKit (WKWebView), the engine the Mac app uses.
3. Serves the emails' images and other requests from a test server on `127.0.0.1`, on a free port.
4. Records every request the page makes and whether any of the email's code ran.

It doesn't launch Moss, open a mailbox, load a model or use any sign-in. The WebKit data store is thrown away after each run.

## The four cases

| Document | Requests allowed | Scripts |
| --- | --- | --- |
| Sanitised, images blocked | None | Removed by the sanitiser |
| Sanitised, images allowed | One image and one background image | Removed by the sanitiser |
| Not sanitised, images blocked | None | Blocked by the Content Security Policy |
| Not sanitised, images allowed | One image and one background image | Blocked by the Content Security Policy |

The two "not sanitised" cases skip the sanitiser on purpose, to show the Content Security Policy holds on its own.

The hostile emails include inline and external scripts, `fetch`, nested frames, objects, remote style sheets, a tracking-size image, a form and a meta refresh.

Every request must be on the exact allowed list, and none may send a referrer. The "images allowed" cases need a real decoded image, so a network failure can't make them pass.

## Output

By default the command prints the number of cases, the test request paths and the result. Set `MOSS_NATIVE_EMAIL_REPORT` to a file path to save the full observations (the file is private, mode `0600`).

## What it doesn't cover

- The packaged app, its IPC bridge, or clicking links in it.
- Popups, the clipboard, file permissions or downloads.
- Every possible hostile HTML or CSS trick. These are fixed cases, not fuzzing.

The remote-image setting and the frame wiring have their own UI tests in `src/design-system/organisms/EmailBody/`.
