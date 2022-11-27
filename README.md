# Rust Video Editor

Shitty, CPU-only, ffmpeg dependant video editor.
Made for automating memes.


### Example

See `examples/basic.rs` for full example.

```rs
let editor = Editor::new(640, 360, Duration::from_secs(10), 25.0)

// Base video layer
.layer(
    Layer::new(
        loader.load_file("assets/sample.m4v")?.layer_data()?,
        Duration::ZERO,
        Transform::ZERO
    )
    .effect(Effect::ScaleToBase { force: true })
)

// Sample image overlay
.layer(
    Layer::new(
        loader.load_file("assets/sample.png")?.layer_data()?,
        Duration::from_secs(5),
        Transform::Percent(0.5, 0.5)
    )
    // Add bad effects
    .effect(Effect::ScaleOverTime { x0: 1.0, y0: 1.0, x1: 2.0, y1: 2.0 })
    .effect(Effect::RotateOverTime { a0: 0.0, a1: PI, uncropped: true })
)

// Add audio layer
.layer(
    Layer::new(
        loader.load_file("assets/sample.mp3")?.layer_data()?,
        Duration::from_secs(5),
        Transform::ZERO
    )
    .speed(0.5)
    .effect(Effect::AudioGain { gain: 0.5 })
);

```