CC ?= cc
CFLAGS = -Wall -Wextra -Werror -std=c11

test: test_timefmt
	./test_timefmt

test_timefmt: test_timefmt.c ../main/core/timefmt.c
	$(CC) $(CFLAGS) -o $@ $^

clean:
	rm -f test_timefmt
.PHONY: test clean
