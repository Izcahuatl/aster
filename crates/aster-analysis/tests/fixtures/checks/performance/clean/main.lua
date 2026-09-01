local sqrt = math.sqrt
local tostring = tostring
local concat = table.concat

local result = {}
for i = 1, 100 do
    local val = sqrt(i)
    result[i] = tostring(val)
end
local s = concat(result, ",")