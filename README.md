# Agility

A powerful and elegant reactive programming library for Rust, inspired by category theory concepts. Agility provides composable, type-safe signals for building reactive systems with both single-threaded and thread-safe variants.

[![Crates.io](https://img.shields.io/crates/v/agility.svg)](https://crates.io/crates/agility)
[![Documentation](https://docs.rs/agility/badge.svg)](https://docs.rs/agility)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

## Features

- **🔄 Reactive Signals**: Fine-grained reactive primitives with automatic dependency tracking
- **🧵 Thread-Safe Variant**: `SignalSync` for concurrent programming with `Send + Sync` support
- **📦 Composable Operations**: Rich API with `map`, `combine`, `extend`, and category-theory-inspired operations
- **🎯 Type-Safe**: Leverages Rust's type system for compile-time guarantees
- **⚡ Efficient**: Smart batching prevents redundant reactions during multiple updates
- **🔗 Weak/Strong References**: Control memory management with flexible reference strategies
- **🏗️ Derive Macros**: Automatically lift structs containing signals with `#[derive(Lift)]` and `#[derive(LiftSync)]`
- **🎭 Category Theory Concepts**: `contramap`, `promap` for bidirectional data flow

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
agility = "0.1.0"
```

## Quick Start

```rust
use agility::Signal;

// Create a signal with an initial value
let counter = Signal::new(0);

// Map the signal to create a derived signal
let doubled = counter.map(|x| x * 2);

// Observe changes with strong references
doubled.with(|x| println!("Counter doubled: {}", x));

// Update the signal - observers are notified automatically
counter.send(5); // Prints: "Counter doubled: 10"
```

## Core Concepts

### Signals

A `Signal<'a, T>` represents a reactive value that can change over time. When a signal's value changes, all dependent signals are automatically updated.

```rust
use agility::Signal;

let temperature = Signal::new(20);
let fahrenheit = temperature.map(|c| c * 9 / 5 + 32);

fahrenheit.with(|f| println!("Temperature: {}°F", f));
temperature.send(25); // Prints: "Temperature: 77°F"
```

### Weak vs Strong References

Agility provides two strategies for managing signal lifetimes:

- **`map()`**: Creates derived signals with **weak references**
  - The derived signal doesn't keep the source alive
  - **Important**: You must keep a binding (`let _observer = ...`) for reactions to fire
  - Without a binding, the signal is immediately dropped and won't propagate changes
  
- **`with()`**: Creates derived signals with **strong references**
  - The derived signal keeps the source alive
  - The binding keeps everything in the dependency chain alive
  - Use when you need guaranteed lifetime management

```rust
let source = Signal::new(10);

// ❌ Wrong: reaction never fires (immediately dropped)
source.map(|x| println!("Value: {}", x));

// ✅ Correct: keep the binding alive
let _observer = source.map(|x| println!("Value: {}", x));

// ✅ Strong reference: also keeps the binding
source.with(|x| println!("Value: {}", x));
```

### Batching Updates

Signal guards enable batching multiple updates to prevent redundant reactions:

```rust
let a = Signal::new(1);
let b = Signal::new(2);
let sum = a.combine(&b).map(|(x, y)| x + y);

sum.with(|total| println!("Sum: {}", total));

// Batch updates - reaction fires only once
(a.send(10), b.send(20)); // Prints: "Sum: 30" (only once)
```

## Advanced Features

### Combining Signals

Combine multiple signals into compound values:

```rust
use agility::Signal;

let first_name = Signal::new("John".to_string());
let last_name = Signal::new("Doe".to_string());

let full_name = first_name.combine(&last_name)
    .map(|(first, last)| format!("{} {}", first, last));

full_name.with(|name| println!("Full name: {}", name));
first_name.send("Jane".to_string()); // Prints: "Full name: Jane Doe"
```

### Lifting Collections

Lift arrays or vectors of signals into a single signal:

```rust
use agility::{Signal, LiftInto};

let x = Signal::new(1);
let y = Signal::new(2);
let z = Signal::new(3);

// Lift array of signals
let coords = [&x, &y, &z].lift();
coords.with(|[a, b, c]| println!("Coordinates: ({}, {}, {})", a, b, c));

x.send(10); // Prints: "Coordinates: (10, 2, 3)"

// Lift tuple of signals
let point = (&x, &y).lift();
point.with(|(a, b)| println!("Point: ({}, {})", a, b));
```

### Extending Signals

Extend a signal with additional signals to create a vector:

```rust
let first = Signal::new(1);
let second = Signal::new(2);
let third = Signal::new(3);

let all = first.extend(vec![second, third]);
all.with(|values| println!("All values: {:?}", values));

first.send(10); // Prints: "All values: [10, 2, 3]"
```

### Category Theory Operations

#### Contravariant Mapping

Flow data backwards from derived to source:

```rust
let result = Signal::new(42);
let source = result.contramap(|x| x * 2);

result.with(|x| println!("Result: {}", x));
source.with(|x| println!("Source: {}", x));

source.send(100); // Prints: "Source: 100" then "Result: 200"
```

#### Profunctor (Bidirectional) Mapping

Create bidirectional data flow between signals:

```rust
let celsius = Signal::new(0);
let fahrenheit = celsius.promap(
    |c| c * 9 / 5 + 32,  // Forward: C -> F
    |f| (f - 32) * 5 / 9  // Backward: F -> C
);

celsius.with(|c| println!("Celsius: {}", c));
fahrenheit.with(|f| println!("Fahrenheit: {}", f));

celsius.send(100);     // Prints both values
fahrenheit.send(32);   // Prints both values (0°C)
```

### Signal Dependencies

Make one signal depend on another:

```rust
let master = Signal::new(10);
let follower = Signal::new(0);

follower.depend(&master);
follower.with(|x| println!("Follower: {}", x));

master.send(42); // Prints: "Follower: 42"
```

## Thread-Safe Signals

For concurrent programming, use `SignalSync`:

```rust
use agility::SignalSync;
use std::thread;

let counter = SignalSync::new(0);
let doubled = counter.map(|x| x * 2);

doubled.with(|x| println!("Value: {}", x));

let counter_clone = counter.clone();
thread::spawn(move || {
    counter_clone.send(10);
}).join().unwrap();
// Prints: "Value: 20"
```

## Derive Macros

Automatically lift structs containing signals:

```rust
use agility::{Signal, Lift};

#[derive(Lift)]
struct AppState<'a> {
    counter: Signal<'a, i32>,
    name: String,
}

let state = AppState {
    counter: Signal::new(0),
    name: "App".to_string(),
};

let lifted = state.lift(); // Signal<'a, _AppState>
lifted.with(|s| println!("Counter: {}, Name: {}", s.counter, s.name));
```

For thread-safe structs, use `#[derive(LiftSync)]`:

```rust
use agility::{SignalSync, LiftSync};

#[derive(LiftSync)]
struct ThreadSafeState<'a> {
    value: SignalSync<'a, i32>,
    label: String,
}
```

## Performance Considerations

- **Automatic Cleanup**: Weak references allow unused signals to be garbage collected
- **Batch Updates**: Use tuples `(signal1.send(x), signal2.send(y))` to batch updates
- **Strong References**: Use `with()` and `and()` when you need to keep signals alive
- **Thread Safety**: `SignalSync` uses `Arc`, `Mutex`, and `RwLock` for thread-safe operations

## Comparison with Other Libraries

| Feature | Agility | Other Reactive Libs |
|---------|---------|-------------------|
| Weak References | ✅ Built-in | ❌ Usually not supported |
| Thread-Safe Variant | ✅ `SignalSync` | ⚠️ Varies |
| Category Theory Ops | ✅ `contramap`, `promap` | ❌ Rare |
| Derive Macros | ✅ Auto-lift structs | ⚠️ Limited |
| Batch Updates | ✅ Signal guards | ⚠️ Manual |
| Type Safety | ✅ Compile-time | ✅ Varies |

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

## Inspiration

Agility is inspired by:
- Reactive programming concepts from functional languages
- Category theory (functors, contravariant functors, profunctors)
- Fine-grained reactivity systems like SolidJS and Leptos
- The need for a flexible, composable reactive library in Rust

## Changelog

### 0.1.0 (Initial Release)
- Single-threaded `Signal` with automatic dependency tracking
- Thread-safe `SignalSync` for concurrent programming
- Rich API with map, combine, extend operations
- Category theory operations: contramap, promap
- Derive macros for automatic struct lifting
- Weak and strong reference strategies
- Batch update support with signal guards
