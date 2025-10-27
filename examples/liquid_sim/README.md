# 🌊 Liquid Simulation

A high-performance CPU-accelerated particle-based fluid simulation that demonstrates the complete GSH rendering pipeline with compression and network transfer.

## Features

This example showcases all the steps outlined in the issue:

1. **🧱 CPU Frame Buffer Creation**: Creates an RGBA8 format frame buffer in memory
2. **⚙️ Parallel Simulation**: Uses multi-threaded CPU compute (via rayon) to simulate particle physics with:
   - Gravity
   - Inter-particle forces
   - Wall collisions with damping
   - Simple fluid-like behavior
3. **📦 Direct Memory Access**: Direct access to pixel data in clean row-major format
4. **💾 CPU Rendering**: Renders particles directly into RGBA8 image data
5. **🚀 Compressed Transfer**: Compresses frames with Zstd before sending through GSH for bandwidth optimization

## Technical Implementation

### CPU Pipeline

- **rayon**: Data-parallel computation for particle updates
- **ndarray**: Efficient 2D array operations for frame buffer
- **glam**: Vector math for physics calculations
- **Parallel Updates**: All particle positions and forces computed simultaneously

### Particle Physics

The simulation implements:
- Gravitational force pulling particles downward
- Velocity damping for realistic motion
- Elastic collisions with boundaries
- Repulsive forces between nearby particles (fluid-like behavior)
- Velocity-based particle coloring (blue = slow, cyan/white = fast)

### Compression

- **Zstd**: High-quality compression algorithm (~50-70% size reduction)
- Level 3 compression (good balance between speed and ratio)
- Automatic decompression on GSH client side

### Data Flow

```
CPU Particle Simulation (rayon) → CPU Rendering (ndarray) → RGBA8 Buffer
                                                                  ↓
                                                            Zstd Compression
                                                                  ↓
                                                            GSH Network Protocol
```

## Building and Running

```bash
# Build the example
cargo build --release -p liquid_sim

# Run the server
cargo run --release -p liquid_sim

# In another terminal, connect with the GSH client
gsh localhost
```

## Performance

- **Particle Count**: 2048 particles
- **Resolution**: 512x512 (resizable)
- **Target FPS**: 60 FPS
- **CPU Usage**: Multi-threaded parallel processing
- **Compression Ratio**: ~50-70% typical (depends on scene complexity)

## Dependencies

- **rayon** (1.10.0): Data parallelism for CPU compute
- **ndarray** (0.16.1): Efficient array operations
- **glam** (0.29.2): Vector and matrix math
- **libgsh**: GSH server framework (includes Zstd compression)

## Notes

- This is a CPU-based version demonstrating the complete pipeline
- The simulation is intentionally simple to keep focus on the GSH integration
- For GPU acceleration, the same pipeline concept applies but with wgpu for compute shaders
- The example gracefully handles window resizing by reinitializing the simulation

## Future Enhancements

Possible improvements:
- Port to GPU with wgpu compute shaders for higher particle counts
- Implement SPH (Smoothed Particle Hydrodynamics) for more realistic fluid behavior
- Add user interaction (mouse/touch to disturb particles)
- Implement metaball rendering for smooth liquid surface
- Add multiple fluid types with different properties
- Optimize particle count with spatial hashing for O(n) neighbor search
