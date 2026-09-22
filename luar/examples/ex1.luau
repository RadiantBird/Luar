local Cat = {}
Cat.__index = Cat

function Cat.new()
    local self = setmetatable({}, Cat)
    self.name = "Fluffy"
    self.name = "Floppa"
    print(`A new cat named {self.name} is born`)
    return self
end

function Cat.meow(self)
    print("Meow!")
end

function Cat.work(self)
    print("The cat is working hard!")
end
local RoyalCat = setmetatable({}, { __index = Cat })
RoyalCat.__index = RoyalCat

function RoyalCat.new()
    local self = setmetatable({}, RoyalCat)
    self.name = "Sir Fluffy"
    self.name = "Sir Floppa"
    print(`A royal cat named {self.name} is born`)
    return self
end

function RoyalCat.meow(self)
    print("Meow, I am a royal cat!")
end

function RoyalCat.laugh(self)
    print("Nehehehe!")
end
local cat1 = Cat.new()
local cat2 = RoyalCat.new()
cat1:meow()
cat2:meow()
cat1:work()
cat2:laugh()