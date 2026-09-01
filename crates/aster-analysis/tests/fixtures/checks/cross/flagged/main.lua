local player = require("player")

print(player.nope)
player:missing()
local ok = player.name


local x = player.move(1, 2)
local a, b = player.move(1, 2)

local y = player:move(1, 2)
