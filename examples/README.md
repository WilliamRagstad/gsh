# 🎓 Example Gallery

This directory showcases two example GSH (Graphical Shell) services built with [`libgsh`](https://github.com/gsh-shell/libgsh). Each service demonstrates a unique rendering pipeline powered by Rust and pixel-level control over the display.

<br/>

## Authentication

|                       Title                        | Description                                                                                                                                                                                            |
| :------------------------------------------------: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
|  <a href="password_auth/"><h3>🔑 Password</h3></a>  | A simple password authentication example. It demonstrates how to use the `gsh` library to create a basic authentication system with a password prompt.                                                 |
| <a href="signature_auth/"><h3>🔑 Signature</h3></a> | A more advanced authentication example that uses a digital signature to verify the identity of the user. It demonstrates how to use cryptographic signatures to authenticate users in a secure manner. |

<br/>

## Rendering

|                                                    Title                                                    | Description                                                                                                                                                                                                                                                                                                                                                                          |
| :---------------------------------------------------------------------------------------------------------: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
|      <a href="cube/"><h3>🧊 Spinning 3D Cube</h3></a> <img src="cube/preview.gif" alt="Spinning Cube"/>      | A real-time software-rendered 3D cube that rotates smoothly and responds to window resizing. This example highlights: <br/> <ul><li>Matrix-based 3D transformations (<code>vek</code>)</li><li>Dynamic perspective projection</li><li>Framebuffer rendering</li><li>Fullscreen window with <code>resize_frame = true</code></li><li>Continuous rotation without user input</li></ul> |
| <a href="colors/"><h3>🎨 Random Color Generator</h3></a> <img src="colors/preview.gif" alt="Random Colors"/> | A playful example showing how to generate and render random colors on user interaction. Two windows display the current and previous colors. <br/> <ul><li>User-driven input handling</li><li>Fixed-size dual windows</li><li>Efficient RGBA buffer construction</li></ul>                                                                                                           |
