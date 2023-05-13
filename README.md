# How to use

Clone the repo, the then run

```sh
cargo install --path .
```

Make sure `libpdfium.dylib` (macOS) or `libpdfium.so` (Linux) is copied to
`/usr/local/lib/`. (I'll write a proper build script at some point, but
currently this is just for me).

Tested in WezTerm (with macOS), but it should also work with Kitty.
