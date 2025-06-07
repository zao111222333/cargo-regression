#include <iostream>

int main() {
    int *p = nullptr;
    std::cout << "About to dereference a null pointer...\n";
    *p = 42;
    std::cout << "This line will never be reached.\n";
    return 0;
}