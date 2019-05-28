#include "chunk.h"
#include "debug.h"
#include "run.h"
#include "utils.h"
#include "vm.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAJOR_VERSION 0
#define MINOR_VERSION 1
#define PATCH_VERSION 0

int main(int argc, char* argv[]) {
  char* binaryName = argv[0];

  if (argc > 1) {
    char* command = argv[1];

    switch (hash(command)) {
      case 210708092629: // "build"
      case 177671:       // "b"
        printf("'build' not yet implemented.");
        return 1;

      case 6385651512: // "repl"
      case 177687:     //"r"
        if (argc == 2) {
          return runRepl();
        }

        return runFile(argv[2]);

      case 229486327000139: //"version"
      case 177691:          //"v"
        printf("v%d.%d.%d\n", MAJOR_VERSION, MINOR_VERSION, PATCH_VERSION);
        return 0;

      case 229469891348124: //"install"
      case 177678:          //"i"
        printf("'install' not yet implemented.");
        return 1;

      case 6385723493: //"test"
      case 177689:     //"t"
        printf("'test' not yet implemented.");
        return 1;

      case 6385292014: //"help"
      case 177677:     //"h"
        break;

      default:
        fprintf(stderr,
                "Unknown command: %s\n"
                "\n"
                "For a list of available commands, run:\n"
                "  %s help\n",
                command, binaryName);

        return 1;
    }
  }

  printf("%s - v%d.%d.%d\n"
         "\n"
         "Compiler & related tools for the Hum language\n"
         "\n"
         "USAGE:\n"
         "  %s <command> [flags...]\n"
         "\n"
         "COMMANDS:\n"
         "  build, b    Compile program from entry point\n"
         "  test, t     Run project tests\n"
         "  repl, r     Start interactive interpreter\n"
         "  install, i  Install dependencies\n"
         "  version, v  Print the compiler version\n"
         "  help, h     Print this help text\n",
         binaryName, MAJOR_VERSION, MINOR_VERSION, PATCH_VERSION, binaryName);

  return 0;
}