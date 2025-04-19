# 🎨 Random Color Generator

![Preview](preview.gif)

## Overview

This GSH (Graphical Shell) service demonstrates a simple real-time visual that responds to user interaction by generating and displaying random colors in two windows.

## Features

- Click to generate a new random color
- Displays the current and previous color side-by-side

## Technical Details

- **Rendering**: RGBA pixel buffer manually filled per frame
- **Interaction**: Any user input triggers a new color generation
- **Windows**: Two fixed-size windows, centrally anchored
  - `Colors!` shows the current color
  - `Previous` shows the last color
