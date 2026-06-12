import re


def line(i):
    a = (i * 7) % 100000
    b = (i * 13) % 100000
    c = (i * 31) % 100000
    return f"user={a} noise word here id={b} and ok={c}"


text = "\n".join(line(i) for i in range(12000))
pat = re.compile(r"([A-Za-z]+)=([0-9]+)")
count = 0
total = 0
for m in pat.finditer(text):
    count += 1
    total = (total + m.end() - m.start()) % 1000000007
print(count)
print(total)
