def in_set(cr, ci):
    zr = 0.0
    zi = 0.0
    for _ in range(1000):
        zr2 = zr * zr
        zi2 = zi * zi
        if zr2 + zi2 > 4.0:
            return 0
        zi = 2.0 * zr * zi + ci
        zr = zr2 - zi2 + cr
    return 1


def main():
    width = 150
    dx = 2.5 / width
    dy = 2.5 / width
    count = 0
    for py in range(width):
        ci = -1.25 + py * dy
        for px in range(width):
            cr = -2.0 + px * dx
            count += in_set(cr, ci)
    print(count)


main()
