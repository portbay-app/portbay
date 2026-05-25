# Developer Certificate of Origin

PortBay uses the [Developer Certificate of Origin](https://developercertificate.org/)
(DCO) instead of a Contributor License Agreement. The DCO is a lightweight,
per-commit certification that you have the right to submit your contribution
under the repository's license (**AGPL-3.0-only**). There is no form to sign and
no account to create — you certify each commit with a sign-off line.

## How to sign off

Add a `Signed-off-by` trailer to every commit by passing `-s`:

```bash
git commit -s -m "feat(dns): add wildcard resolver health check"
```

This appends a line using your configured `git` name and email:

```
Signed-off-by: Jane Developer <jane@example.com>
```

Use a real name and a reachable email. To set them once:

```bash
git config user.name  "Jane Developer"
git config user.email "jane@example.com"
```

Forgot to sign off the last commit? Amend it:

```bash
git commit --amend -s --no-edit
```

To sign off a range of existing commits, rebase with `--signoff`:

```bash
git rebase --signoff main
```

## What you are certifying

> By contributing to PortBay, you certify that you have the right to submit the
> contribution and that it can be licensed under the repository's applicable
> license (AGPL-3.0-only).

The full text of the certificate follows.

---

```
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.
1 Letterman Drive
Suite D4700
San Francisco, CA, 94129

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.


Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

---

> This document explains a process. It is not legal advice.
