# Agility

A library for advanced responsive programming with procedural macro support.

## Project Structure

This is a Cargo workspace containing:
- `agility` - Main library with signals and reactive programming primitives
- `agility-macros` - Procedural macros for the agility library

## Procedural Macros

The project includes several example procedural macros:
- `#[derive(SimpleDebug)]` - Derive macro for simple debug formatting
- `make_answer!()` - Function-like macro that generates code
- `#[log_call]` - Attribute macro for logging function calls

See `agility-macros/README.md` for more details.

## Testing

### Running All Tests
```bash
cargo test
```

### Running Proc Macro Tests Only
```bash
cargo test --test proc_macro_tests
```

### Running Trybuild Tests (compile-time validation)
```bash
cargo test --test trybuild_tests
```

### Running Integration Tests in the Workspace
```bash
cargo test --workspace
```
