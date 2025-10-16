# Agility Macros

This crate contains procedural macros for the `agility` project.

## Available Macros

### `#[derive(SimpleDebug)]`
A derive macro that implements a simple debug trait for your types.

```rust
use agility::{SimpleDebug, SimpleDebug as SimpleDebugDerive};

#[derive(SimpleDebugDerive)]
struct MyStruct;

let s = MyStruct;
println!("{}", s.simple_debug()); // Prints: MyStruct()
```

### `make_answer!()`
A function-like macro that generates a function returning 42.

```rust
use agility::make_answer;

make_answer!();

assert_eq!(answer(), 42);
```

### `#[log_call]`
An attribute macro that logs when functions are called.

```rust
use agility::log_call;

#[log_call]
fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}

greet("World");
// Prints: "Calling function: greet"
// Prints: "Function greet returned"
```

## Testing

This crate is tested through integration tests in the main `agility` crate.
