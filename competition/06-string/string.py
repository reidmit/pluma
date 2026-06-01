parts = ["item-" + str(i) for i in range(50000)]
joined = ",".join(parts)
back = joined.split(",")
upper = joined.upper()
print(len(back))
print(len(joined))
print(len(upper))
