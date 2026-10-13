---
name: swing-publish
description: Check a static site before it is published with SWING (`swing publish`, the Nostr + IPFS mirror of personal sites) and hand the user the command to publish it. Use when the user wants to publish or update their site with SWING, asks whether their site will work on IPFS gateways and mirrors, or asks why `swing publish` warns or stops. Only runs `swing publish --dry-run`; never publishes.
---

# Check a site for `swing publish`

This skill was written for swing {{version}}. `swing publish <DIR>` adds a built static site to IPFS, signs a site event with the user's Nostr key and sends it to the user's relays. Other SWING users then keep copies of every version and serve them through IPFS gateways. Your job is to make the site work as a plain directory served by a gateway, confirm it with the dry run, and hand the final command to the user.

## The rule: never publish

- Never run `swing publish` without `--dry-run`, in any form: not the binary, not `docker compose run ... mirror publish`, not the dashboard's Publish screen. A publish cannot be undone (mirrors keep old versions and anyone can copy IPFS data) and it is signed with the user's key.
- Do not create or change keys or swing's configuration (`swing key generate`, `swing signer pair`, `swing.toml`, `.env`), and do not loosen a check mode (`--check-* off`, `--nip05 off`) just to make the dry run pass. Suggest such changes to the user and let them decide.
- Fix problems in the site's source (templates, config, assets), not in the build output, and only after the user agrees to the change you propose.
- When everything is checked, show the publish command for the user to run themselves, and stop.

## Step 1: Find how swing is installed

1. Run `swing --version`. If it works, swing is a binary on the PATH. If its version is not {{version}}, tell the user that this skill belongs to another version and that `swing skill install` (or `swing skill show`) gives the matching one.
2. Otherwise look for a Docker Compose setup: a `compose.yaml` with a `mirror` service, usually in a clone of the SWING repository. Ask the user where it is if you cannot find it. Run swing there as `docker compose run --rm -T mirror <args>`. Mount the site's build output into the container and pass the path inside it:

   ```bash
   docker compose run --rm -T -v "/abs/path/to/public:/site:ro" mirror publish --site example.jp --url https://example.jp/ --dry-run --json /site
   ```

   `-T` keeps the container's stderr out of stdout, so the JSON stays parseable. File paths in the result are relative to the site directory.
3. If neither works, ask the user how they run swing. Do not install it yourself.

The dry run does not need `swing up` to be running, a key, or a reachable relay. The local checks (dotfiles, size, links) always run; whatever needs Kubo, the key or the relays is reported as unknown instead of failing. With a binary, swing reads the same configuration as the user's `swing up` (`--config`, `SWING_CONFIG`, or the default location). If the user runs swing with `--config <path>`, pass the same option.

## Step 2: Find the site and its build output

- Work out the static site generator and its build command from the project (`package.json` scripts, `hugo.toml`, `_config.yml`, `astro.config.*`, and so on).
- Find the build output directory (`public/`, `dist/`, `_site/`, `build/`, `out/` ...). Only this directory is published. Never pass the repository root: everything in the directory is added, including dotfiles, and swing refuses directories that contain its own configuration or data.
- Build the site with the project's usual command before every dry run.

## Step 3: Decide the arguments

Ask the user rather than guess. The same arguments are used for the dry run and for the real publish.

- `--site` (required) becomes the site identifier `d`. Mirrors show it as the site's name and it should never change. Use the user's domain name (e.g. `example.jp`) when they control the root of that domain; otherwise any stable identifier (e.g. `example-com-notes`). If they have published before, reuse the same value: `swing replicas` without arguments lists the user's own sites with their `d=`.
- `--url` is the site's HTTP(S) address, if it is also served on the web. Omit it for an IPFS-only site. With `--url`, the link check also reports absolute URLs that point back to that site.
- NIP-05: when `--site` is a domain, swing checks `https://<domain>/.well-known/nostr.json` for `{"names": {"_": "<hex public key>"}}`. The hex key is `pubkey.hex` in the dry run's JSON. The file can be part of the site's output (`.well-known` is allowed by the dotfile check). If the user does not control the domain root, use a non-domain `--site` or `--nip05 off`.
- `--title` sets a display title. `-m` / `--message` is an update note shown to people who mirror the site and in Nostr clients; describe what changed in this version. swing rejects a message that is too long before doing anything.
- `--note` (needs `--url`) also posts a normal Nostr note (kind 1) about the update after publishing.
- Leave the check modes (`--nip05`, `--check-dotfiles`, `--check-size`, `--check-links`, `--check-unchanged`) at the user's configuration unless they ask otherwise.

## Step 4: Run the dry run

```bash
swing publish --site example.jp --url https://example.jp/ -m "Add a post about ..." --dry-run --json ./public
```

Read the exit code and the streams together:

| Exit code | stdout | Meaning |
|---|---|---|
| 0 | JSON | The publish would go ahead (`outcome` is `publish` or `unchanged`). |
| 1 | JSON | The publish would stop. Every reason is in `blockers`; stderr repeats them after `the publish would stop:`. |
| 1 | empty | An error before the checks started: a bad argument (`invalid --site` ...), the config file, a protected path (swing's config, state or Kubo repository inside the directory), or the directory could not be listed. Read the `Error:` line on stderr, fix it and run again. |
| 2 | empty | A command-line usage error (e.g. `--json` without `--dry-run`); see stderr. |

Without `--json` the same result is printed for humans. Use `--json` yourself and show the user a summary.

## Step 5: Read the JSON

First check that `format` is `1`. If it is another number, the schema has changed since this skill was written: tell the user to update the skill with `swing skill install` and read the fields with care.

Conventions:

- A value swing could not find out is an object with `"status": "unknown"` and a `detail` with the reason. `null` means the value does not exist (no `--url`, no previous version), which is different from unknown.
- Unknown never stops the publish. Typical causes are no key configured yet, Kubo not running (`swing up` is not running), or relays that cannot be reached. Report them; start services or change settings only if the user asks.
- Each check has `mode` (`off`, `warn`, `require`) and `stops` (`true` if this check alone would stop the real publish).
- Never copy limits or defaults from this skill or from memory; use the values in the JSON (for example `checks.size.guideline`).

Fields:

- `outcome`: `publish` (would be published), `unchanged` (same content as the latest version; the real publish exits 0 without publishing), `blocked` (would stop).
- `blockers`: array of `{check, message}`; `check` is one of `nip05`, `dotfiles`, `size`, `links`, `clock`.
- `site`, `url`, `title`, `message`, `note`: the arguments as swing understood them.
- `pubkey`: `{status: "known", hex, npub}` or unknown.
- `nip05`: `status` is `off`, `verified`, `mismatch`, `not_applicable` (the `d` is not a domain), `error` or `unknown`, with `detail` for `error` and `unknown`.
- `checks.dotfiles`: `status` `off`/`ok`/`found`, `count`, `paths` (every file or directory whose name starts with `.`). Remove them from the build output (e.g. `.DS_Store`, `.git`); if one is really meant to be public, the user can add its name to `[publish].dotfiles_allow`.
- `checks.size`: `status` `off`/`ok`/`over`, `files`, `bytes` (total of all files), `guideline`. It is only a guideline: each mirror applies its own limits, and a smaller site is kept by more of them. Shrink it if it is over (Step 6).
- `checks.links`: `status` `off`/`ok`/`found`, `count`, `breaking` (how many certainly break), `redirects` (whether a top-level `_redirects` exists), `skipped` (files too large to read, so not checked), `findings`. Each finding has `kind`, `breaks`, `file` (where the reference is), `reference`, `files` (how many files contain it). Kinds:
  - `reserved` (breaks): a top-level file or directory named `ipfs` or `ipns`. Rename it.
  - `root_relative` (breaks): a reference starting with `/`, including `<base href="/">`. Make it relative to the page (`css/style.css`, `../about/`).
  - `broken` (breaks): the referenced file is not in the site. Fix the path or its case, or remove the reference (e.g. an unused `@font-face` format).
  - `insecure_script` (breaks): `<script src="http://...">`. Bundle the script.
  - `post_form`: a form that is not GET; it cannot work on a gateway.
  - `worker`: Service Worker or Web Worker registration; blocked on mirrors' gateways. The page must work without it.
  - `insecure_request`: `http://` or `ws://` URLs in scripts or form actions; use `https://`/`wss://` or remove.
  - `external`: a resource loaded from another host. Bundle it (Step 6).
  - `own_site`: an absolute URL to the `--url` site. Make it relative unless it is meant to point at the original site.
- `clock`: `status` `ok` or `stops`, `created_at` (the date the new version would get), `detail`, `mfs_versions`, `relays_answered`. If it stops, tell the user what `detail` says. It may suggest a command that deletes versions from swing's storage; never run that yourself.
- `ipfs`: `{status: "computed", cid}` or unknown. The CID is the one the real publish would produce.
- `previous`: the user's latest version on the relays: `{status: "found", cid, created_at}`, `{status: "none"}` or unknown.
- `new_files`: `status` `listed`/`no_previous`/`unknown`, `compared_with`, `count`, `paths` (files that were not in the previous version; with no previous version, every file). Read every path and point out anything that should not be public (drafts, backups, private data, keys, databases, exports). The real publish asks the user to confirm these.
- `unchanged`: `status` `changed`/`unchanged`/`no_previous`/`unknown`, `previous_cid`, `skips_publish` (true if the real publish would end without publishing because nothing changed).
- `guide_url`: the site guide for this version of swing.

## Step 6: Check what the dry run cannot see

The dry run only checks what it can find mechanically. Go through the rest of the checklist by reading the site's source and its build output. It also cannot see URLs that scripts build at run time, references inside SVG files, `<meta http-equiv="refresh">`, or the files listed in `checks.links.skipped`.

1. **Fonts, CSS and JavaScript from a CDN.** Look for `https://` and `//` in `<link>`, `<script>`, `@import`, `@font-face` and `url(...)` in the output and in templates, and for fonts or libraries loaded by a theme. Download them into the site, after checking their licenses.
2. **Images and video hosted elsewhere.** Copy the files into the site. For embeds (video players, social posts, maps), add a plain link or still image so the page still reads without them.
3. **External APIs and server-side features.** Comments, contact forms, site search, `fetch` to an API, analytics and ads do not work, or do nothing useful, on a mirror. The main content must be readable without them. Requests over `http://` and `ws://` are blocked on mirrors' gateways.
4. **Absolute URLs to the user's own domain.** Search the output for the domain. Links inside the site must be relative. `<link rel="canonical">` and `og:url` may stay absolute because they are meant to name the original.
5. **Extensionless URLs and server rewrites.** A gateway serves files as they are: `/about` does not find `about.html`. Links must name `about.html`, or `about/` with an `about/index.html`. `.htaccess`, `_headers`, `netlify.toml`, `vercel.json` and other host settings have no effect. A single-page app needs hash routing (`#/about`) or one HTML file per page. `_redirects` works only on some gateways; nothing essential should depend on it.
6. **Case of file names.** Gateways are case-sensitive, but Windows and macOS usually are not, so a link to `image.png` for `Image.PNG` works locally and breaks on a mirror. Compare references with the real names, including ones built by scripts. Lowercase names avoid the problem.
7. **Source maps, sources and `node_modules`.** Look for `*.map`, `node_modules/`, unbuilt sources (`*.ts`, `*.scss`, `*.md` templates), and build config in the output. Turn them off in the build or exclude them.
8. **Secrets and private data.** If a secret scanner is installed, run it on the output directory (`gitleaks dir <dir>`, `trufflehog filesystem --fail <dir>`, `detect-secrets scan --all-files <dir>`); otherwise suggest one to the user rather than installing it. Also look for files such as `*.pem`, `*.key`, `id_*`, `*.sql`, `*.env`, `secrets.*`, backups and drafts, and for location data in photos. Anything published stays copied on other people's machines.
9. **Size of images, video and fonts.** List the largest files. Suggest resizing images to their display size and converting them to WebP or AVIF, shortening or moving long videos out of the site, and subsetting fonts (a full Japanese font is several MB). Large files that are regenerated on every build are downloaded again by every mirror on every update.
10. **Reproducible builds.** Build twice into two directories and compare them with `diff -r`. Differences usually come from build timestamps, random file names or query strings, lists in unstable order (archives, sitemaps, feeds) or re-encoded images. Every difference changes the CID, so an unchanged site would look like a new version.
11. **Things the user has no rights to.** Publishing lets every SWING participant store, copy and serve the site. Point out third-party text, images, fonts and libraries, and ask the user to check that their licenses allow this.

Propose each fix to the user, change the source only after they agree, rebuild, and run the dry run again.

## Settings of static site generators

Hints for the problems above. Option names change between versions: confirm each one in the official documentation of the version the project uses before changing it, and say so if you could not.

- **Hugo**: `relativeURLs = true` makes internal links relative. Pretty URLs (the default) write `about/index.html`; `uglyURLs = true` writes `about.html` instead. Avoid `absURL` for internal links in templates.
- **Jekyll**: the `relative_url` filter produces paths that start with `/` (with `baseurl`), which are root-relative. Write page-relative paths in templates, or find a plugin or post-processing step for relative links in the Jekyll docs. Check that `permalink` gives each page an `index.html` or an `.html` name that matches the links.
- **Eleventy**: pages are written as `about/index.html` by default; links written as `/about/` are root-relative. Write relative links or look in the Eleventy docs for a way to make them relative.
- **Astro**: `build.format` decides between `about/index.html` and `about.html`, and `trailingSlash` how links end. Built assets are referenced from `/_astro/`, which is root-relative; check the Astro docs and integrations for relative output.
- **Vite**: `base: './'` makes the URLs of built assets relative. A router using the History API needs hash mode.
- **Next.js**: `output: 'export'` produces a static site, `trailingSlash: true` writes `about/index.html`, and `next/image` needs `images.unoptimized: true`. Assets live under `/_next/`, which is root-relative; check the Next.js docs on `assetPrefix` and `basePath`.
- **Docusaurus**: `trailingSlash` decides how pages are written. Its client-side router expects to be served from `baseUrl`; check the Docusaurus docs for a hash router or other options for serving from any path.
- **Any other tool**: look for options named like "relative URLs", "base URL/path", "trailing slash", "pretty/clean URLs" and "static export".

## Step 7: Hand over the publish command

When the dry run exits 0 and you have gone through Step 6:

1. Summarize: `outcome`, the CID, the new files, the warnings that remain (with the user's decision on each), and the values that were unknown.
2. If `outcome` is `unchanged`, there is nothing to publish; say so.
3. Show the command to run: the same arguments without `--dry-run` and `--json`. When files are new, the real publish lists them and asks `y/N`; the user should read the list. Then stop. Do not run it.

Other ways the user can publish:

- The dashboard's Publish screen (open it with `swing dashboard open`) takes a folder from the browser. Its first button, Check contents, only runs the checks (NIP-05, dotfiles, size, links and new files) without uploading anything; the user then reviews the results and presses Publish to upload and sign.
- In CI, the real publish needs `--yes` because there is no terminal to confirm new files. Exit code 0 means published or unchanged; anything else means it failed. Run the build, a secret scan and the dry run first, and stop the job if any of them fails.
- With Docker Compose, drop `-T`, `--dry-run` and `--json` from the command in Step 1.

## Reference

These pages describe swing {{version}}; when they disagree with this skill, they are right.

- Site checklist and reasons: https://github.com/amane-katagiri/swing/blob/v{{version}}/docs/site-guide.md
- Publishing (arguments, NIP-05, dry run, dashboard): https://github.com/amane-katagiri/swing/blob/v{{version}}/docs/guide/publish.md
- Dry-run output, JSON schema and exit codes: https://github.com/amane-katagiri/swing/blob/v{{version}}/docs/architecture/cli/publish.md
- Settings and their defaults: https://github.com/amane-katagiri/swing/blob/v{{version}}/docs/guide/operation.md
