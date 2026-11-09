# Security policy

## Supported versions

Before 1.0, only the latest released minor version gets security fixes.

## Reporting a vulnerability

Please **don't open a public issue** for a security problem. Report it
privately with the **Report a vulnerability** button on the repository's
[Security tab](https://github.com/abedmurrar/yace/security). Include:
- the YACE version (`yace --version`)
- your OS and terminal emulator
- steps to reproduce
- the impact

You'll get an acknowledgement within 7 days. A fix or mitigation plan
follows once the report is triaged.

## Scope and security model

- **YACE runs locally.** It sends no telemetry and never sends file
  contents anywhere. Its only network use is downloading the plugin
  index and a plugin, and only when you ask ("Extensions: Browse
  Plugins…" or `yace --install-plugin <id>`): over HTTPS with `curl`, and
  a download is refused unless its SHA-256, id and version match the
  index.
- **Opening a workspace never runs anything.** `tasks`, `launch` and other
  executable configuration in `.code-workspace` files are ignored.
- **Native plugins are not sandboxed.** A plugin loaded into the YACE
  process can do anything YACE can, so only install plugins you trust.
  Before installing, YACE shows the plugin's identity, publisher, declared
  capabilities and the SHA-256 of its code, and asks you to confirm. It
  records those hashes and loads a plugin's code only while they still
  match, so a plugin changed on disk stays unloaded until you approve it
  again. `yace --safe-mode` starts without loading any
  plugin code.
- **Logs are opt-in and contain no file contents.** Logs are written only
  when `YACE_LOG` is set.

Things that count as vulnerabilities include: data loss or corruption on
save, code running just because a workspace was opened, and file contents
or credentials leaking into logs or to other processes.
