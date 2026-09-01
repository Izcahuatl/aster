local function three() return 1, 2, 3 end

local a, b, c = three()
print(three())
local t = { three() }
local x = external()
local y = (three())

local function cond(v)
    if v then return 1 end
    return 1, 2
end
local p, q = cond()
