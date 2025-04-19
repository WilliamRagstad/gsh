# 🧊 Spinning 3D Cube – GSH Example

![Preview](preview.gif)

## Overview

This example demonstrates a GSH (Graphical Shell) service that renders a spinning 3D cube in real-time. The cube remains centered and smoothly adjusts to window resizing.

## Features

- Real-time software rendering using RGBA pixel buffers
- Dynamic rotation with adjustable speed
- Responsive to window resizing while maintaining center alignment
- Built with Rust and the [`vek`](https://crates.io/crates/vek) math library

## Technical Details

- **Rendering**: Software-based, utilizing `vek::Mat4` for transformations
- **Projection**: Simple perspective projection with adjustable depth
- **Window Management**: Configured via GSH protocol with `allow_resize` and `FrameAnchor::Center`
- **Interaction**: Continuous rotation; no user input required

## Getting Started

1. Ensure you have Rust installed.
2. Clone the repository and navigate to the project directory.
3. Build and run the project with `cargo run`, then connect to the GSH service using a compatible client.

	```bash
	gsh localhost --insecure
	```

4. The application will open a window displaying the spinning cube.
5. Resize the window to see the cube remain centered and adjust its size accordingly.
