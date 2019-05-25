#include "chunk.h"
#include "memory.h"
#include "value.h"
#include <stdlib.h>

void initChunk(Chunk* chunk) {
  chunk->count = 0;
  chunk->capacity = 0;
  chunk->code = NULL;

  chunk->lineCount = 0;
  chunk->lineCapacity = 0;
  chunk->lines = NULL;

  initValueArray(&chunk->constants);
}

void freeChunk(Chunk* chunk) {
  FREE_ARRAY(uint8_t, chunk->code, chunk->capacity);
  FREE_ARRAY(LineStart, chunk->lines, chunk->lineCapacity);
  freeValueArray(&chunk->constants);
  initChunk(chunk);
}

void writeChunk(Chunk* chunk, uint8_t byte, int line) {
  if (chunk->capacity < chunk->count + 1) {
    int oldCapacity = chunk->capacity;
    chunk->capacity = GROW_CAPACITY(oldCapacity);
    chunk->code = GROW_ARRAY(chunk->code, uint8_t, oldCapacity, chunk->capacity);
  }

  chunk->code[chunk->count] = byte;
  chunk->count++;

  if (chunk->lineCount > 0 && chunk->lines[chunk->lineCount - 1].line == line) {
    return;
  }

  if (chunk->lineCapacity < chunk->lineCount + 1) {
    int oldCapacity = chunk->lineCapacity;
    chunk->lineCapacity = GROW_CAPACITY(oldCapacity);
    chunk->lines = GROW_ARRAY(chunk->lines, LineStart, oldCapacity, chunk->lineCapacity);
  }

  LineStart* lineStart = &chunk->lines[chunk->lineCount++];
  lineStart->offset = chunk->count - 1;
  lineStart->line = line;
}

int addConstant(Chunk* chunk, Value value) {
  writeValueArray(&chunk->constants, value);

  return chunk->constants.count - 1;
}

int getLine(Chunk* chunk, int instructionOffset) {
  int idx = 0;

  // TODO binary search?
  for (;;) {
    LineStart* line = &chunk->lines[idx];

    if (idx == chunk->lineCount - 1 || instructionOffset < chunk->lines[idx + 1].offset) {
      return line->line;
    }

    idx++;
  }
}