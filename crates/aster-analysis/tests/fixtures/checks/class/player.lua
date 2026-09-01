local Entity = require("entity")

local Player = {}
Player.__index = Player
setmetatable(Player, { __index = Entity })

function Player.new(name)
    local self = Entity.new(name)
    self.score = 0
    return setmetatable(self, Player)
end

function Player:level()
    return self.score
end

return Player
