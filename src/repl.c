#include <stdio.h>

int runRepl() {
  char line[1024];

  for (;;) {
    printf("> ");

    if (!fgets(line, sizeof(line), stdin)) {
      printf("\n");
      break;
    }

    printf("you said: %s", line);
  }

  return 0;
}