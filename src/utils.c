const unsigned long hash(const char* str) {
  unsigned long hash = 5381;

  int ch;
  while ((ch = *str++)) {
    hash = ((hash << 5) + hash) + ch;
  }

  return hash;
}