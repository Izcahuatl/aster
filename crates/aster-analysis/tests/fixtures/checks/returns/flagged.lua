local function three() return 1, 2, 3 end
local function one() return 1 end
local function none() end

local x = three()
local a, b = three()
local p, q = one()
local y = none()

local function outer()
    local function inner() return 1 end
    return inner
end

local z1, z2 = outer()
