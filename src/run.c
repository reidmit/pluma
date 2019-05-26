#include "vm.h"
#include <stdio.h>

static int run(char* source) {
  initVM();
  InterpretResult result = interpret(source);
  freeVM();

  if (result == INTERPRET_COMPILE_ERROR) {
    return 65;
  }

  if (result == INTERPRET_RUNTIME_ERROR) {
    return 70;
  }

  return 0;
}

int runRepl() {
  char line[1024];

  for (;;) {
    printf("> ");

    if (!fgets(line, sizeof(line), stdin)) {
      printf("\n");
      break;
    }

    run(line);
  }

  return 0;
}

int runFile(char* file) {
  printf("running file: %s", file);
  return 0;
}