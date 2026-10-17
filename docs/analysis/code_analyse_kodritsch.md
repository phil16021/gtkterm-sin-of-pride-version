# Code-Analyse Dokumentation

**Student:** Philipp
**Kurs:** Test & Verification
**Dozent:** Martin Horauer
**Datum:** 10. November 2025
**GitHub Repository:** https://github.com/phil16021/gtkterm-sin-of-pride-version

---

## 1. Projektübersicht

**Analysiertes Projekt:** GTKTerm - Serial Terminal Applikation für Linux
**Projektgröße:** ca. 20.000 Lines of Code in C
**Hauptkomponenten:** Serial/TCP/UDP Kommunikation, GTK4 GUI, Custom Visualisierung ("Sunshine")

---

## 2. Verwendete Analyse-Tools

### Statische Analyse
- **gcc** (v13.3.0) mit `-Wall -Wextra --pedantic`
- **clang-tidy** (v18.1.3) für erweiterte statische Code-Analyse

### Dynamische Analyse
- **valgrind** (v3.22.0) für Memory-Leak Detection
- **gprof** (v2.42) für Performance-Profiling

---

## 3. Durchgeführte Analysen

### 3.1 Statische Analyse

**Ergebnisse:**
- 14.419 Warnings generiert (hauptsächlich aus GTK/GLib Headers)
- 3 relevante Compiler-Warnings im eigenen Code gefunden
- GObject-Pattern Warnings (akzeptabel, da GTK-Standard)

### 3.2 Dynamische Analyse

**Valgrind Test-Programm (`test_memory.c`):**
```bash
gcc -g -Wall -Wextra test_memory.c -o test_memory
valgrind --tool=memcheck --leak-check=yes -s ./test_memory leak
```

**Gprof Test-Programm (`test_performance.c`):**
```bash
gcc -pg -g test_performance.c -o test_performance
./test_performance 4
gprof ./test_performance gmon.out > performance_analysis.txt
```

---

## 4. Findings und Interpretation

### 4.1 Statische Analyse - Findings

| Problem | Datei | Zeile | 
|---------|-------|-------|
| Dead Code: `gtkterm_preset_clone()` | gtkterm_presets.c | 40 |
| Unused Variable: `plot_width` | sunshine/bar_chart.c | 309 |
| Unused Variable: `zero_y` | sunshine/bar_chart.c | 408 | 
| Unbenannte Funktionsparameter | Diverse | - | 
| GObject-Pattern Warnings | Diverse | - | 

**Interpretation:**

**Dead Code (gtkterm_presets.c:40):**
Die Funktion `gtkterm_preset_clone()` wird definiert aber nie aufgerufen. Dies ist typischer "Dead Code" der unnötig Speicher belegt und Wartung erschwert.

**Unused Variables (bar_chart.c):**
`plot_width` wird berechnet aber nicht verwendet. `zero_y` war vermutlich für negative Werte geplant, wurde aber nie implementiert. Solche Variablen können zu Verwirrung führen.

**GObject-Pattern Warnings:**
Identifikatoren wie `_GtkTermBufferClass` mit führendem Unterstrich sind laut C-Standard reserviert. In GTK/GObject-Code ist dies jedoch Standard und akzeptabel.

### 4.2 Dynamische Analyse - Findings

**Valgrind Memory-Leak Detection:**
```
HEAP SUMMARY:
  in use at exit: 100 bytes in 1 blocks
  total heap usage: 2 allocs, 1 frees, 4,196 bytes allocated

100 bytes in 1 blocks are definitely lost
  at malloc (vg_replace_malloc.c)
  by leaky_function (test_memory.c:8)
```

**Interpretation:**
Valgrind erkennt Memory-Leaks zuverlässig. Im Test-Programm wurde malloc() ohne entsprechendes free() verwendet. Bei langlebigen Anwendungen wie GTKTerm können solche Leaks zu erheblichen Problemen führen.

**Valgrind Use-After-Free Detection:**
```
Invalid write of size 1
  at invalid_access (test_memory.c:33)
Address 0x4a8c080 is 0 bytes inside a block of size 100 free'd
  at free (vg_replace_malloc.c)
  by invalid_access (test_memory.c:32)
```

**Interpretation:**
Zugriff auf bereits freigegebenen Speicher ist kritisch und kann zu Crashes und Sicherheitslücken führen.

**Gprof Performance-Profiling:**
```
  %   cumulative   self              self     total
 time   seconds   seconds    calls   s/call   s/call  name
 93.23      1.24     1.24        1     1.24     1.24  sw_delay_flat
  6.02      1.32     0.08     4096     0.00     0.00  delay
  0.75      1.33     0.01   100000     0.00     0.00  factorial_recursive
  0.00      1.33     0.00   100000     0.00     0.00  factorial_iterative
```

**Interpretation:**
Die rekursive Implementierung erzeugt 2.000.000 Funktionsaufrufe (100.000 direkt + 1.900.000 rekursiv), während die iterative Variante nur 100.000 Aufrufe benötigt. Dies zeigt den erheblichen Overhead von Rekursion.


