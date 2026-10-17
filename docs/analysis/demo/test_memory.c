/**
 * Testprogramm für Valgrind Memory-Fehler-Erkennung
 *
 * Dieses Programm demonstriert verschiedene Arten von Speicherfehlern,
 * die mit Valgrind erkannt werden können.
 *
 * Kompilierung: gcc -g -Wall -Wextra test_memory.c -o test_memory
 * Ausführung:   valgrind --tool=memcheck --leak-check=yes -s ./test_memory [test]
 *
 * Tests: leak, bounds, uninit, invalid
 */
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

/**
 * Funktion mit absichtlichem Memory Leak
 *
 * Problem: malloc() wird aufgerufen, aber free() fehlt
 * Valgrind-Erkennung: "definitely lost: 100 bytes in 1 blocks"
 */
void leaky_function(void) {
    char *buffer = malloc(100);
    // FEHLER: Kein free() - Memory Leak!
    strcpy(buffer, "This memory will be leaked");
    printf("Allocated 100 bytes\n");
    // Hier sollte stehen: free(buffer);
}

/**
 * Funktion mit Out-of-Bounds Zugriff
 *
 * Problem: Zugriff auf array[10], aber array hat nur Größe 10 (Indizes 0-9)
 * Valgrind-Erkennung: Stack-basierte Arrays werden nicht immer erkannt
 * (Hinweis: Heap-basierte Out-of-Bounds werden besser erkannt)
 */
void bounds_violation(void) {
    char array[10];
    array[10] = 'X';  // FEHLER: Out of bounds - gültig sind nur array[0] bis array[9]
    printf("Wrote to array[10] (out of bounds)\n");
}

/**
 * Funktion mit Verwendung einer uninitialisierten Variable
 *
 * Problem: Variable x wird deklariert aber nicht initialisiert
 * Valgrind-Erkennung: "Conditional jump or move depends on uninitialised value(s)"
 */
void uninitialized_use(void) {
    int x;  // FEHLER: Nicht initialisiert - enthält "Garbage"
    if (x == 42) {  // FEHLER: Verwendung einer uninitialisierten Variable
        printf("x equals 42\n");
    }
    // Hier sollte stehen: int x = 0; oder eine andere Initialisierung
}

/**
 * Funktion mit Use-After-Free Fehler
 *
 * Problem: Zugriff auf Speicher nach free()
 * Valgrind-Erkennung: "Invalid write of size 1" + "Address is X bytes inside a block of size Y free'd"
 * Dies ist ein kritischer Fehler, der zu Crashes und Sicherheitslücken führen kann!
 */
void invalid_access(void) {
    char *ptr = malloc(100);
    free(ptr);
    ptr[0] = 'A';  // FEHLER: Use after free - Zugriff auf freigegebenen Speicher!
    printf("Modified freed memory\n");
    // Richtig wäre: ptr = NULL; nach dem free(), um weitere Zugriffe zu verhindern
}

int main(int argc, char *argv[]) {
    printf("=== Memory Error Demonstration ===\n\n");

    if (argc > 1) {
        if (strcmp(argv[1], "leak") == 0) {
            printf("Test 1: Memory Leak\n");
            leaky_function();
        }
        else if (strcmp(argv[1], "bounds") == 0) {
            printf("Test 2: Out-of-Bounds Access\n");
            bounds_violation();
        }
        else if (strcmp(argv[1], "uninit") == 0) {
            printf("Test 3: Uninitialized Variable\n");
            uninitialized_use();
        }
        else if (strcmp(argv[1], "invalid") == 0) {
            printf("Test 4: Invalid Access (use after free)\n");
            invalid_access();
        }
        else {
            printf("Usage: %s [leak|bounds|uninit|invalid]\n", argv[0]);
        }
    } else {
        printf("Usage: %s [leak|bounds|uninit|invalid]\n", argv[0]);
    }

    return 0;
}
