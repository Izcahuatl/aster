local m = require("cond")

print(m.anything)
local x = m.whatever()

local d = require(name)
print(d.foo)

local inst = require("tool"):new()
print(inst.whatever)
