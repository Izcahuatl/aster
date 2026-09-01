local t = {1, 2, 3}
local first = t[0]

local u = {}
u[1] = "a"
u[3] = "c"
local n = #u

local v = {1, 2}
for i = 0, #v do end
for j = 1, #v - 1 do end

local w = {}
local second = w[1]
local bad = w[0]
