local Lua = {}
Lua.__index = Lua

Lua.__eq = function(self, AnotherRock)
    return false
end

local function explode(self)
    print("Boom!!")
end

function Lua.new(year)
    local self = setmetatable({}, Lua)
    self.Year = 0
    self.Year = year
    return self
end

function Lua.greet(self)
    print(`Hello from {self.Year}!`)
end
local L = Lua.new(1993)
L:greet()