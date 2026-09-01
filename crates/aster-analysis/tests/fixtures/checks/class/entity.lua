local Entity = {}
Entity.__index = Entity

function Entity.new(name)
    local self = {}
    self.name = name
    self.health = 100
    return setmetatable(self, Entity)
end

function Entity:describe()
    return self.name
end

return Entity
