# Trail-managed Colima runtime toolchain

Trail can download and install the following pinned upstream command-line
tools into a user-owned cache when `trail env runtime setup colima` is run.
They are not linked into Trail and are not stored in the Trail repository.

- Colima 0.10.3, copyright Abiola Ibrahim and contributors, MIT License:
  https://github.com/abiosoft/colima
- Lima 2.2.0, copyright the Lima contributors, Apache License 2.0:
  https://github.com/lima-vm/lima
- Docker CLI 29.7.2, copyright Docker, Inc. and contributors, Apache License
  2.0: https://github.com/docker/cli

`COLIMA-LICENSE` contains Colima's required MIT notice.
`APACHE-2.0-LICENSE` contains the Apache License 2.0 applicable to Lima and
the Docker CLI. The upstream Lima archive also retains its own documentation
and license files below `share/doc/lima`.
