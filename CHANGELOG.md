## Changelog

### 0.1.0 (Initial Release)
- Single-threaded `Signal` with automatic dependency tracking
- Thread-safe `SignalSync` for concurrent programming
- Rich API with map, combine, extend operations
- Category theory operations: contramap, promap
- Derive macros for automatic struct lifting
- Weak and strong reference strategies
- Batch update support with signal guards

### 0.1.1
- Added `and` method to `SignalGuard` and `SignalGuardSync`
- Removed all value cloning. No more need for `T: Clone` bounds
- Used `swap` for value updates to optimize performance
- Better implementation of `depend` methods
- Deduplicated codes