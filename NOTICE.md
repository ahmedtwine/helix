# Notice

This repository is a **fork** of the Helix editor.

- Upstream project: <https://github.com/helix-editor/helix>
- Fork: <https://github.com/ahmedtwine/helix>
- Diverged from upstream at commit `079a789e` (`master`)

## License

Helix is licensed under the **Mozilla Public License 2.0**. This fork keeps that
licence unchanged; see [`LICENSE`](LICENSE) for the full text.

Under MPL-2.0 §3.1 and §3.3, every file in this repository — original and
modified — remains under the MPL-2.0. Modified files are Covered Software and
stay under the same terms. No upstream copyright or licence notice has been
removed or altered.

If you distribute a binary built from this repository, MPL-2.0 §3.2 requires you
to make the corresponding source form available under the MPL-2.0 and to inform
recipients how to obtain it.

## Modifications

Everything this fork adds lives in the `helix-studio` crate, documented in
[`helix-studio/README.md`](helix-studio/README.md). Upstream crates are touched
in a small, deliberately contained set of places, listed in that document's
"Upstream diff" section so the delta stays easy to audit and to rebase.

The `hx` binary in this fork is built by `helix-studio`. The unmodified upstream
binary is still built, as `hx-vanilla`.

## Branches

| Branch | Purpose |
| --- | --- |
| `master` | Mirrors `helix-editor/helix` untouched, so `git merge upstream/master` stays conflict-free |
| `studio` | Default development branch for this fork |

To take upstream changes:

```sh
git fetch upstream
git checkout master && git merge --ff-only upstream/master && git push
git checkout studio && git merge master
```

## Trademarks and endorsement

"Helix" and the Helix logo belong to the Helix project and its contributors. This
fork is **not** affiliated with, endorsed by, or supported by the upstream Helix
project. Please report issues with this fork here, not upstream.
