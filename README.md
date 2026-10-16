# Rustpad

> **About this fork.** Rustpad is the work of
> [Eric Zhang (@ekzhang)](https://github.com/ekzhang), who designed and built it:
> the operational transformation, the server, the editor, all of it. The original
> lives at [ekzhang/rustpad](https://github.com/ekzhang/rustpad).
>
> This fork adds a few small things on top: MySQL as a storage option
> (`DATABASE_URL`, next to the existing SQLite support), document titles, an
> optional private list of documents (behind Cloudflare Access), and dependencies
> kept up to date for security fixes. Everything else here, and all the hard work
> behind it, is Eric's. The badges and links below still point to his project.

[![Docker Pulls](https://img.shields.io/docker/pulls/ekzhang/rustpad)](https://hub.docker.com/r/ekzhang/rustpad/)
[![Docker Image Size](https://img.shields.io/docker/image-size/ekzhang/rustpad/latest)](https://hub.docker.com/r/ekzhang/rustpad/)
[![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/ekzhang/rustpad/ci.yml)](https://github.com/ekzhang/rustpad/actions/workflows/ci.yml)

**Rustpad** is an _efficient_ and _minimal_ open-source collaborative text
editor based on the operational transformation algorithm. It lets users
collaborate in real time while writing code in their browser. Rustpad is
completely self-hosted and fits in a tiny Docker image, no database required.

<p align="center">
<a href="https://rustpad.io/">
<img src="https://i.imgur.com/WjU5UrP.png" width="800"><br>
<strong>rustpad.io</strong>
</a>
</p>

The server is written in Rust using the
[warp](https://github.com/seanmonstar/warp) web server framework and the
[operational-transform](https://github.com/spebern/operational-transform-rs)
library. We use [wasm-bindgen](https://github.com/rustwasm/wasm-bindgen) to
compile text operation logic to WebAssembly code, which runs in the browser. The
frontend is written in TypeScript using [React](https://reactjs.org/) and
interfaces with [Monaco](https://github.com/microsoft/monaco-editor), the text
editor that powers VS Code.

Architecturally, client-side code communicates via WebSocket with a central
server that stores in-memory data structures. This makes the editor very fast,
allows us to avoid provisioning a database, and makes testing much easier. The
tradeoff is that documents are transient and lost between server restarts, or
after 24 hours of inactivity.

## Development setup

To run this application, you need to install Rust, `wasm-pack`, and Node.js.
Then, build the WebAssembly portion of the app:

```
wasm-pack build rustpad-wasm
```

When that is complete, you can install dependencies for the frontend React
application:

```
npm install
```

Next, compile and run the backend web server:

```
cargo run
```

While the backend is running, open another shell and run the following command
to start the frontend portion.

```
npm run dev
```

This command will open a browser window to `http://localhost:5173`, with hot
reloading on changes.

## Testing

To run integration tests for the server, use the standard `cargo test` command.
For the WebAssembly component, you can run tests in a headless browser with

```
wasm-pack test --chrome --headless rustpad-wasm
```

## Configuration

Although the default behavior of Rustpad is to store documents solely in memory
and collect garbage after 24 hours of inactivity, this can be configured by
setting the appropriate variables. The application server looks for the
following environment variables on startup:

- `EXPIRY_DAYS`: An integer corresponding to the number of days that inactive
  documents are kept in memory before being garbage collected by the server
  (default 1 day).
- `SQLITE_URI`: A SQLite connection string used for persistence. If provided,
  Rustpad will snapshot document contents to a local file, which enables them to
  be retained between server restarts and after their in-memory data structures
  expire. (When deploying a Docker container, this should point to the path of a
  mounted volume.)
- `DATABASE_URL`: A MySQL connection string (`mysql://user:pass@host/db`) used
  for persistence instead of SQLite; it takes precedence over `SQLITE_URI`. The
  `document` table is created on startup. Add `?ssl-mode=REQUIRED` to require
  TLS without verifying the server certificate.
- `CF_ACCESS_TEAM_DOMAIN`, `CF_ACCESS_AUD`: Enable a private list of all
  documents at `/documents` (data at `/api/documents`), meant to sit behind a
  [Cloudflare Access](https://developers.cloudflare.com/cloudflare-one/policies/access/)
  application on those two paths. The server checks the signed
  `Cf-Access-Jwt-Assertion` header against the team's keys (for example
  `example.cloudflareaccess.com`) and the application's AUD tag. Without these
  variables (or without a database) both paths return 404. The editor itself
  stays anonymous.
- `CF_ACCESS_ALLOWED_EMAILS`: Optional comma-separated list of email addresses
  allowed to see the list, checked in addition to the Access policy. From the
  list the owner can also set titles (never shown in the editor), lock documents
  (read-only for everyone) and see and restore earlier versions.
- `SNAPSHOT_IDLE_MINUTES`: With a database, a version of a changed document is
  saved once it has not been edited for this many minutes (default 10). A
  version is also saved when a stored document is opened, when it is locked,
  before a restore and at shutdown; identical texts are not saved twice.
- `VERSIONS_KEEP_DAYS`: Versions of the last this many days are all kept
  (default 30); older ones are thinned to the newest of each day.
- `PORT`: Which local port to listen for HTTP connections on (defaults to 3030).
- `RUST_LOG`: Directives that control application logging, see the
  [env_logger](https://docs.rs/env_logger/#enabling-logging) docs for more
  information.

## Deployment

Rustpad is distributed as a single 6 MB Docker image, which is built
automatically from the `Dockerfile` in this repository. You can pull the latest
version of this image from Docker Hub. It has multi-platform support for
`linux/amd64` and `linux/arm64`.

```
docker pull ekzhang/rustpad
```

(You can also manually build this image with `docker build -t rustpad .` in the
project root directory.) To run locally, execute the following command, then
open `http://localhost:3030` in your browser.

```
docker run --rm -dp 3030:3030 ekzhang/rustpad
```

This fork runs at code.small-victories.co on the Kubernetes cluster `sv-oke`.
`npm run deploy` ships `origin/main` there with the shared `sv-deploy` script from the
Oracle repo (config in `deploy.env`); `npm run deploy:check` shows what runs. `SV_DEPLOY`
overrides the script's path (default `../Oracle/deploy/sv-deploy`).

## In the media

- **July 11, 2021:** Featured in
  [Console 61 - The open-source newsletter](https://console.substack.com/p/console-61).
- **June 5, 2021:** Front-page
  [Hacker News post](https://news.ycombinator.com/item?id=27408326). Reddit
  discussions in [r/rust](https://www.reddit.com/r/rust/comments/nt4p9f/) and
  [r/programming](https://www.reddit.com/r/programming/comments/nt4ws7/).

<br>

<sup>
All code is licensed under the <a href="LICENSE">MIT license</a>.
</sup>
