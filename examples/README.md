# 🎓 Example Gallery

This directory showcases two example GSH (Graphical Shell) services built with [`libgsh`](https://github.com/gsh-shell/libgsh). Each service demonstrates a unique rendering pipeline powered by Rust and pixel-level control over the display.

## [🧊 Spinning 3D Cube](cube/)

![Spinning Cube](cube/preview.gif)

A real-time software-rendered 3D cube that rotates smoothly and responds to window resizing. This example highlights:

- Matrix-based 3D transformations (`vek`)
- Dynamic perspective projection
- Framebuffer rendering
- Fullscreen window with `resize_frame = true`
- Continuous rotation without user input

## [🎨 Random Color Generator](colors/)

![Random Colors](colors/preview.gif)

A playful example showing how to generate and render random colors on user interaction. Two windows display the current and previous colors.

- User-driven input handling
- Fixed-size dual windows
- Efficient RGBA buffer construction
