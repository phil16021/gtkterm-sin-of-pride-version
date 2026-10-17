/* Test program for demonstrating gprof performance profiling */
#include <stdio.h>
#include <stdlib.h>

/* Simulate a time-consuming operation */
void delay(void) {
    for (int i = 65535; i > 0; i--) {
        // Busy wait
    }
}

/* Function that uses flat delay */
void sw_delay_flat(unsigned int delay_count) {
    printf("Inside sw_delay_flat\n");
    for (unsigned int i = 0; i < delay_count; i++) {
        // Busy wait
    }
}

/* Function that calls delay multiple times */
void sw_delay_times(unsigned int times) {
    printf("Inside sw_delay_times\n");
    for (unsigned int i = 0; i < times; i++) {
        delay();
    }
}

/* Compute factorial recursively (inefficient) */
unsigned long long factorial_recursive(int n) {
    if (n <= 1) {
        return 1;
    }
    return n * factorial_recursive(n - 1);
}

/* Compute factorial iteratively (efficient) */
unsigned long long factorial_iterative(int n) {
    unsigned long long result = 1;
    for (int i = 2; i <= n; i++) {
        result *= i;
    }
    return result;
}

int main(int argc, char *argv[]) {
    printf("=== Performance Profiling Demonstration ===\n\n");

    if (argc != 2) {
        printf("Usage: %s <iterations>\n", argv[0]);
        printf("Example: %s 4\n", argv[0]);
        exit(EXIT_FAILURE);
    }

    int iterations = atoi(argv[1]);

    printf("Running flat delay (CPU intensive)...\n");
    sw_delay_flat(4294967040);

    printf("Running delay times (function call overhead)...\n");
    for (int i = 0; i < iterations; i++) {
        sw_delay_times(1024);
    }

    printf("Computing factorials recursively...\n");
    for (int i = 0; i < 100000; i++) {
        factorial_recursive(20);
    }

    printf("Computing factorials iteratively...\n");
    for (int i = 0; i < 100000; i++) {
        factorial_iterative(20);
    }

    printf("\nDone! Check gprof output for performance analysis.\n");
    return 0;
}
