#let unused() = { let max = calc.max }
#for i in range(5000) { context counter("x").update(counter("x").final().first() + 1) }
