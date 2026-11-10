# What
Static Wayland wallpapers.

## rustbg
```
rustbg [-f|--fill] [-n <name> | --namespace <name> | --namespace=<name>] <image_path>
```
Sets the wallpaper using the wlr-layer-shell protocol. Image loading and decoding is handled by [dump-bgra](https://github.com/spaceavenue/dump-bgra) (a tool I created specifically to dump bgra pixels as fast as possible. yknow. for Speed(tm)). You need to have it installed on your system for the wallpaper to work. If you don't want to, you can build with `--feature ffmpeg` to use ffmpeg instead. The rest is the same.
- Use `-f / --fill` to fill/center-crop the wallpaper, omit for a "fit" mode where the empty space is padded with black pixels.
- Use `-n <name> / --namespace <name> / --namespace=<name>` to specify the layer surface's namespace.

# Building
The musl target is needed for building. This is subject to change however, since it doesnt actually link against the C runtime or use libc functions.

NOTE: This is a personal project, so don't expect stability or the best code quality :P
Stick to something stable and tested, with more features like [swaybg](https://github.com/swaywm/swaybg), or for a more minimal option, [wbg](https://codeberg.org/dnkl/wbg).

# Why And How
Fun :3  
But also I kinda just went down a rabbit hole of trying to make a wallpaper daemon that consumes as little memory as possible.  

I started out by trying to call `malloc_trim(0)` to free unused heap memory, to then linking with `musl` statically, to then trying to remove `core::fmt` and free myself from Rust's formatting engine, just so I could make the binary itself as small as possible. 
I then shifted to removing any and all formatting done by Rust at `panic!` by enabling `panic-immediate-abort` and, well, immediately aborting on panic. The binary is gutted of location details and debug formatting architecture too, via `-Z location-detail=none` and `-Z fmt-debug=none`.  

The stdlib was then built at compile-time with `panic_abort` (needed for `panic-immediate-abort`) and the feature `optimize-for-size`. I then took on a major architectural shift by removing all external crates and relying only on corelib and libc, and communicating with the server only by reading from and writing to the wayland socket.
It was around this time that the wayland core was split into `wllib` and versioned separately. After a bit, I made the decision to get rid of libc and the C runtime entirely by using the `rustix` crate, filling in the gaps for syscalls like `fork()`, `execve()` and `signal()` myself by writing inline assembly.
