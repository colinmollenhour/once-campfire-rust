# This fork

A fork of [once-campfire-rust](https://github.com/basecamp/once-campfire-rust) with a few features
of its own, kept so that merging upstream stays a routine job.

- Fork-only code goes in `crates/fork` (`campfire_fork`), with its tests.
- Upstream files change only at the hook points below. A hook is one line where possible, ends in
  a `// fork:` comment when the file takes comments, and calls into `campfire_fork`.
- The fork's changes are documented here, not under "Known differences" in `README.md`, which
  stays upstream's list of how the port differs from Rails.
- Fork commits start with `fork:`, so `git log --grep '^fork:'` lists them.

## Hook points

| File | Hook |
|---|---|
| `Cargo.toml` | `campfire_fork` in `[workspace.dependencies]` |
| `crates/campfire/Cargo.toml` | `campfire_fork.workspace = true` |
| `crates/campfire/src/app.rs` | `campfire_fork::init()?` in `run()`, after `Config::from_env()` |
| `crates/campfire/src/controllers/presenters.rs` | `campfire_fork::link_references(html)` in `Presenter::content` |
| `README.md` | the pointer to this file, after the introduction |
| `AGENTS.md` (`CLAUDE.md` links to it) | the "This fork" section at the end |

`git grep -n 'fork:' -- crates` should find one line for each hook in a `.rs` file.

## Features

### References

`CAMPFIRE_REFERENCES` links references to other tools in text messages when they're shown:
GitLab merge requests (`!2989` in a default project, `supersaas!12` in the default group,
`group/project!12` anywhere) and ids with configured prefixes (`DEV-3400`, `SERVER-537`). It's a
JSON array of rules:

```json
[
  {"gitlab": "https://gitlab.example.com", "group": "acme", "project": "server"},
  {"prefixes": ["DEV", "OPS"], "url": "https://app.clickup.com/t/123/{id}"},
  {"prefixes": ["SERVER"], "url": "https://acme.sentry.io/issues/{id}/"}
]
```

`group` and `project` are optional; a `project` needs a `group`. Prefixes are uppercase, and
`{id}` in the URL is replaced by the whole id (`DEV-3400`).

- Text in links, code, code blocks, mentions and attachments is left alone.
- The stored body, plain text (search, push, bot webhooks) and the editor keep what was typed.
- An invalid value stops the app at boot, with the rule that's wrong.
- It's read at boot, so a change takes a restart. The restart also empties the in-process
  fragment store, so no message keeps links made with the old rules.
- Unset or blank, nothing changes.
- Overlapping matches are checked with an ordered interval index, preserving rule priority without
  comparing every reference with all earlier matches. In an optimized standalone run on this host,
  80,000 references in 480 KB took 34 ms, down from 2.44 s. This measures the linker alone, not a
  complete message request; raw before-and-after output is kept in ignored `tmp/bench/`.

The rules are process-wide, set once by `campfire_fork::init`, rather than a field of `Config`
passed down to the presenter. That keeps the hooks to one line each, outside the parts of
`config.rs` upstream changes most. The cost is a global: code that presents messages without
`init` having run (upstream's tests) links nothing.

## Deploying

Production is the `Campfire` service in Railway's Prancing Pony project, built from this fork
rather than pulled from upstream's image:

```sh
railway link     # once: Prancing Pony, production
crates/fork/bin/deploy-railway Campfire            # HEAD, or name a commit
```

It sets `APP_VERSION` and `GIT_REVISION` to the commit, uploads a clean export (Railway builds the
Dockerfile, about 10 minutes), and returns once the upload is done; Railway keeps the old
deployment serving until the new one passes `/up`. The only change to the build is the cache
mount ids, which Railway wants prefixed with the service's id. `CAMPFIRE_REFERENCES` is a
service variable. To roll back, redeploy an earlier deployment from Railway's dashboard.

## Merging upstream

Once, in a clone:

```sh
git remote add upstream git@github.com:basecamp/once-campfire-rust.git
git config rerere.enabled true   # reuse resolutions of conflicts seen before
```

Each time:

```sh
git fetch upstream
git switch -c merge-upstream-$(date +%F) main
git merge upstream/main
```

Merge, don't rebase: `main` is deployed and shared, and a merge keeps the fork's commits as they
were. After resolving conflicts:

1. Check every hook point in the table above is still there and still in the right place. Upstream
   may have moved the code a hook sits in (`Presenter::content`, `run()`) or renamed what it calls.
2. Take upstream's pin of the `reference/` submodule as it is.
3. Push and let CI run: it runs upstream's checks and `crates/fork`'s tests with them.
4. Update this file if a hook moved.
