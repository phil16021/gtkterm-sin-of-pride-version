# Code Analysis Tools - Setup and Usage Guide

This document demonstrates static and dynamic code analysis tools installed and tested on the GTKTerm project.

## Installed Tools

### Static Analysis Tools
- **clang-tidy** (v18.1.3) ✓ Installed
- **cppcheck** ✗ Not installed (requires: `sudo apt-get install cppcheck`)

### Dynamic Analysis Tools
- **valgrind** (v3.22.0) ✓ Installed - Memory error detection
- **gprof** (v2.42) ✓ Installed - Performance profiling

---

## Static Analysis

### 1. Clang-Tidy

**Purpose**: Catches bugs, style issues, and performs static analysis using LLVM infrastructure.

**Usage**:
```bash
clang-tidy -checks="-*,clang-analyzer-*,bugprone-*,cert-*,readability-*" \
    src/gtkterm_buffer.c -- \
    -I./include -I/usr/include/gtk-4.0 \
    -I/usr/include/glib-2.0 \
    -I/usr/lib/x86_64-linux-gnu/glib-2.0/include
```

**Findings on GTKTerm**:
- Reserved identifier warnings (GObject patterns - expected)
- Unnamed parameters in function declarations
- Casting through void pointers (GObject macros)
- 14,419 warnings generated (many from GTK/GLib headers)

**Example Issues Found**:
```
src/gtkterm_buffer.c:97: warning: all parameters should be named
src/gtkterm_buffer.c:124: warning: do not cast through 'void *'
```

### 2. Cppcheck (Not Installed)

**Installation**:
```bash
sudo apt-get install cppcheck
```

**Usage**:
```bash
cppcheck --enable=all --std=c11 src/gtkterm_buffer.c
```

**What it checks**:
- Out-of-bounds array access
- Memory leaks (missing free())
- Uninitialized variables
- Dead code
- Null pointer dereferences
- Style and performance issues

---

## Dynamic Analysis

### 3. Valgrind - Memory Error Detection

**Purpose**: Detects memory leaks, use-after-free, out-of-bounds access, and uninitialized variables at runtime.

**Build Requirements**:
```bash
gcc -g -Wall -Wextra test_memory.c -o test_memory
```
Note: `-g` flag is required for line number information

**Usage**:
```bash
valgrind --tool=memcheck --leak-check=yes -s ./test_memory leak
```

**Test Results**:

#### Memory Leak Test:
```
==29564== 100 bytes in 1 blocks are definitely lost in loss record 1 of 1
==29564==    at 0x4846828: malloc (vgpreload_memcheck-amd64-linux.so)
==29564==    by 0x1091FE: leaky_function (test_memory.c:8)
==29564==    by 0x109358: main (test_memory.c:43)

LEAK SUMMARY:
   definitely lost: 100 bytes in 1 blocks
```

**Conclusion**: Successfully detected memory leak at test_memory.c:8

#### Use-After-Free Test:
```
==29595== Invalid write of size 1
==29595==    at 0x1092E3: invalid_access (test_memory.c:33)
==29595==  Address 0x4a8c080 is 0 bytes inside a block of size 100 free'd
==29595==    at 0x484988F: free (vgpreload_memcheck-amd64-linux.so)
==29595==    by 0x1092DE: invalid_access (test_memory.c:32)
```

**Conclusion**: Successfully detected use-after-free error at test_memory.c:33

**Additional Valgrind Options**:
- `--show-reachable=yes` - Show all unpaired malloc/free calls
- `--track-origins=yes` - Track origin of uninitialized values
- `--verbose` - More detailed output

### 4. Gprof - Performance Profiling

**Purpose**: Identifies performance bottlenecks by measuring time spent in each function.

**Build Requirements**:
```bash
gcc -pg -g -Wall -Wextra test_performance.c -o test_performance
```
Note: `-pg` flag enables profiling instrumentation

**Usage**:
```bash
# 1. Run the program (generates gmon.out)
./test_performance 4

# 2. Analyze with gprof
gprof ./test_performance gmon.out > performance_analysis.txt

# 3. View results
cat performance_analysis.txt
```

**Test Results**:

#### Flat Profile:
```
  %   cumulative   self              self     total
 time   seconds   seconds    calls   s/call   s/call  name
 93.23      1.24     1.24        1     1.24     1.24  sw_delay_flat
  6.02      1.32     0.08     4096     0.00     0.00  delay
  0.75      1.33     0.01   100000     0.00     0.00  factorial_recursive
  0.00      1.33     0.00   100000     0.00     0.00  factorial_iterative
```

**Key Findings**:
1. **sw_delay_flat** consumed 93.23% of runtime - clear bottleneck
2. **factorial_recursive** called 2,000,000 times (100K initial + 1.9M recursive)
3. **factorial_iterative** called only 100,000 times - demonstrates recursion overhead
4. **delay** called 4,096 times from sw_delay_times

#### Call Graph Insights:
```
[1] main → sw_delay_flat (93.2% time)
         → sw_delay_times → delay (6.0% time)
         → factorial_recursive (0.8% time)
         → factorial_iterative (0.0% time)
```

**Optimization Suggestions**:
- Focus optimization efforts on `sw_delay_flat` (highest impact)
- Consider replacing recursive implementations with iterative ones
- Reduce call frequency for frequently-called functions

---

## GTKTerm-Specific Analysis

### Build with Debug Symbols:
```bash
# Configure with debug build type
meson setup build --buildtype=debug

# Compile
ninja -C build
```

**Compilation Warnings Found**:
```
src/gtkterm_presets.c:40: warning: 'gtkterm_preset_clone' defined but not used
src/sunshine/bar_chart.c:309: warning: unused variable 'plot_width'
src/sunshine/bar_chart.c:408: warning: unused variable 'zero_y'
```

### Running Valgrind on GTKTerm:
```bash
valgrind --tool=memcheck --leak-check=yes --track-origins=yes \
    ./build/src/gtkterm-sin-of-pride-version
```
Note: GTKTerm is a GUI application and may generate many GTK/GLib-related reports

---

## Summary

### Static Analysis Benefits:
- Catches bugs before runtime
- Enforces coding standards
- No performance overhead
- Can analyze code paths not covered by tests

### Dynamic Analysis Benefits:
- Catches runtime-only issues (memory leaks, race conditions)
- Provides actual execution profiles
- Detects issues with real data
- Essential for performance tuning

### Best Practices:
1. **Integrate into build process** - Run analysis tools automatically
2. **Use debug builds** - Always compile with `-g` for dynamic analysis
3. **Address warnings early** - Small issues can indicate bigger problems
4. **Profile before optimizing** - Measure, don't guess
5. **Run valgrind regularly** - Memory leaks compound over time

### Tool Selection Guide:

| Issue Type | Tool | When to Use |
|------------|------|-------------|
| Memory leaks | valgrind | After major changes, before release |
| Performance bottlenecks | gprof | When performance issues suspected |
| Code quality | clang-tidy | During development, in CI/CD |
| MISRA/CERT compliance | cppcheck | Safety-critical systems |
| Out-of-bounds access | valgrind | Integration testing |
| Uninitialized variables | valgrind | After refactoring |

---

## Test Files Created

- `test_memory.c` - Demonstrates valgrind capabilities
- `test_performance.c` - Demonstrates gprof profiling
- `performance_analysis.txt` - gprof output

## References

- Clang-Tidy: https://clang.llvm.org/extra/clang-tidy/
- Valgrind Manual: https://valgrind.org/docs/manual/manual.html
- Gprof Manual: https://sourceware.org/binutils/docs/gprof/
- Cppcheck: http://cppcheck.net/
