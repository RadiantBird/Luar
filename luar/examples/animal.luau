local Animal = {}
Animal.__index = Animal

function Animal.new(n)
    local self = setmetatable({}, Animal)
    self.name = "unknown"
    self.name = n
    return self
end

function Animal.speak(self)
end
local Dog = setmetatable({}, { __index = Animal })
Dog.__index = Dog

function Dog.new(n)
    local self = Animal.new(n)
    setmetatable(self, Dog)
    return self
end

function Dog.speak(self)
    print("Woof!")
end
local d = Dog.new("Rex")
d:speak()