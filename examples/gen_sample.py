#!/usr/bin/env python3
"""Generate examples/sample.vcd (a small counter / FSM / UART-ish design)."""
import math, os

out = []
w = out.append
w("$date 2026-11-11 $end\n$version terminalwave sample generator $end\n$timescale 1ns $end")
w("$scope module tb $end")
w("$var wire 1 ! clk $end\n$var wire 1 \" rst_n $end")
w("$scope module dut $end")
w("$var wire 1 ! clk $end\n$var wire 1 \" rst_n $end")
w("$var reg 8 # count [7:0] $end")
w("$var reg 2 $ state [1:0] $end")
w("$var wire 16 % data [15:0] $end")
w("$var wire 1 & valid $end")
w("$var wire 1 ' tx $end")
w("$var wire 4 ( nibble [3:0] $end")
w("$var real 64 ) analog $end")
w("$var wire 32 * addr [31:0] $end")
w("$upscope $end\n$upscope $end\n$enddefinitions $end")
w("#0\n$dumpvars\n0!\n0\"\nbx #\nb00 $\nbz %\n0&\n1'\nbxxxx (\nr0 )\nb0 *\n$end")
count = 0; state = 0; data = 0
for t in range(5, 2001, 5):
    w(f"#{t}")
    clk = (t // 5) % 2
    w(f"{clk}!")
    if t == 25:
        w("1\"")
    if clk == 1 and t > 25:
        count = (count + 1) & 0xFF
        w(f"b{count:b} #")
        if count % 16 == 0:
            state = (state + 1) % 4
            w(f"b{state:b} $")
        if count % 8 == 3:
            data = (data * 31 + 0x1234) & 0xFFFF
            w(f"b{data:b} %\n1&")
        if count % 8 == 4:
            w("0&")
        w(f"b{(count >> 2) & 0xF:b} (")
        if count % 32 == 0:
            w(f"b{count * 0x100 + 0xDEAD0000:b} *")
    if t % 40 == 0:
        w(f"{(t // 40) % 3 % 2}'")
    if t % 10 == 0:
        w(f"r{math.sin(t / 100.0):.4f} )")
    if t == 1500:
        w("bx %")
    if t == 1600:
        w("bz %")
path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sample.vcd")
with open(path, "w") as f:
    f.write("\n".join(out) + "\n")
