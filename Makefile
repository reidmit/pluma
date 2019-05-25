BINARY_NAME = hum

CC = gcc -Wall
src = $(wildcard src/*.c)
obj = $(src:.c=.o)

main: $(obj)
	$(CC) -o $(BINARY_NAME) $^

clean:
	rm -f $(obj) $(BINARY_NAME)

.PHONY: clean
