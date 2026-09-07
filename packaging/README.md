# packaging

Host-integration files that are **not** part of the binary.

## Nothing in here ships in a release

The release job builds one artifact and tars one file:

```
# .github/workflows/release.yml
tar -C dist -czf "${name}.tar.gz" kern
```

So a release tarball contains the `kern` binary and nothing else, and `cargo install` copies one
file. **Anything you add to this directory has no delivery path until you give it one.** That is a
property of the release job, not an oversight to route around silently: a user who installed from a
tarball has no checkout, so a message that tells them to run
`something packaging/whatever` fails with "No such file".

This was found the expensive way. `kern doctor` printed
`sudo install -m644 packaging/apparmor/kern /etc/apparmor.d/kern`, which works if you are standing
in a checkout and fails for everyone else, which is most people.

## The three ways out, in the order to try them

1. **Embed it in the binary and emit it.** What `apparmor/kern` does:
   `include_str!` compiles the file in, and `kern doctor --apparmor-profile` writes it to stdout.
   The shipped copy and the emitted copy cannot diverge because they are the same bytes, and the
   command works for every install method. Right for small text files a user needs to install.

2. **Add it to the release artifacts.** Right for anything too large to embed, or that a user needs
   before they have the binary. Costs a change to `release.yml` and a second file for people to
   verify.

3. **Leave it here and reference it by URL**, for files only a contributor needs. Right for nothing
   that an end user is ever told to run.

## Contents

| | what | how it reaches a user |
|---|---|---|
| `apparmor/kern` | AppArmor profile granting `userns`, needed on Ubuntu 23.10+ | embedded, `kern doctor --apparmor-profile` |
