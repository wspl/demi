import { test, assert, assertEq, sleep } from "./harness.mjs";

test("timers: ordering by delay then registration", async () => {
  const order = [];
  setTimeout(() => order.push("b10"), 10);
  setTimeout(() => order.push("a0"), 0);
  setTimeout(() => order.push("c0"), 0);
  setTimeout(() => order.push("d5"), 5);
  await sleep(30);
  assertEq(order, ["a0", "c0", "d5", "b10"]);
});

test("timers: clearTimeout and arguments", async () => {
  let fired = null;
  const id = setTimeout(() => (fired = "no"), 1);
  clearTimeout(id);
  setTimeout((a, b) => (fired = a + b), 2, "x", "y");
  await sleep(10);
  assertEq(fired, "xy");
});

test("timers: setInterval repeats and clears from inside", async () => {
  let n = 0;
  await new Promise((resolve) => {
    const id = setInterval(() => {
      n++;
      if (n === 3) { clearInterval(id); resolve(); }
    }, 2);
  });
  await sleep(10);
  assertEq(n, 3);
});

test("timers: sleep accuracy", async () => {
  const t0 = performance.now();
  await sleep(20);
  const dt = performance.now() - t0;
  assert(dt >= 19 && dt < 60, `slept ${dt}ms`);
});
