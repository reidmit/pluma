TARGET = hum

CFLAGS = -std=c99 -Wall -I.
LFLAGS = -Wall -I. -lm

SOURCES := $(wildcard src/*.c)
OBJECTS := $(SOURCES:src/%.c=obj/%.o)

bin/$(TARGET): $(OBJECTS)
	@mkdir -p bin
	gcc $(OBJECTS) $(LFLAGS) -o $@

$(OBJECTS): obj/%.o : src/%.c
	@mkdir -p obj
	gcc $(CFLAGS) -c $< -o $@

.PHONY: clean
clean:
	@rm -f $(OBJECTS) bin/$(TARGET)
	@echo "Removed: $(OBJECTS) bin/$(TARGET)"