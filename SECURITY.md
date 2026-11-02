# Security policy

## Supported versions

Before 1.0, only the latest released minor version gets security fixes.

## Reporting a vulnerability

Please **don't open a public issue** for a security problem. Report it
privately through GitHub's "Report a vulnerability" button (private
vulnerability reporting) on the repository, once it's published. Include:
- the YACE version (`yace --version`)
- your OS and terminal emulator
- steps to reproduce
- the impact

You'll get an acknowledgement within 7 days. A fix or mitigation plan
follows once the report is triaged.

## Scope and security model

- **YACE runs entirely locally.** It sends no telemetry and makes no
  network requests in normal use. It never sends file contents anywhere.
- **Opening a workspace never runs anything.** `tasks`, `launch` and other
  executable configuration in `.code-workspace` files are ignored (spec
  §4.12).
- **Native plugins (planned, M8) are not sandboxed.** A plugin loaded into
  the YACE process can do anything YACE can. Only install plugins you
  trust. YACE will show a plugin's identity, publisher and integrity
  information before installing it, and a safe mode will start YACE
  without any third-party plugins (spec §18.9, §33.11).
- **Logs are opt-in and contain no file contents.** Logs are written only
  when `YACE_LOG` is set.

Things that count as vulnerabilities include: data loss or corruption on
save, code running just because a workspace was opened, and file contents
or credentials leaking into logs or to other processes.
