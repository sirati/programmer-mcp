x: int = 42

class Greeter:
    def __init__(self, name: str) -> None:
        self.name = name

    def greet(self) -> str:
        return f"Hello, {self.name}!"

def add(a: int, b: int) -> int:
    return a + b

result = add(x, 10)
g = Greeter("world")
print(g.greet())
