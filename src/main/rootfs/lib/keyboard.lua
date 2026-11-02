local keyboard = {}

-- Following OC1's naming, but with Linux key codes.
keyboard.keys = {
  ["1"] = 2, ["2"] = 3, ["3"] = 4, ["4"] = 5, ["5"] = 6, ["6"] = 7, ["7"] = 8, ["8"] = 9, ["9"] = 10, ["0"] = 11,
  a = 30, b = 48, c = 46, d = 32, e = 18, f = 33, g = 34, h = 35, i = 23, j = 36, k = 37, l = 38, m = 50,
  n = 49, o = 24, p = 25, q = 16, r = 19, s = 31, t = 20, u = 22, v = 47, w = 17, x = 45, y = 21, z = 44,

  apostrophe = 40,
  back = 14, -- backspace
  backslash = 43,
  capital = 58, -- caps lock
  comma = 51,
  enter = 28,
  equals = 13,
  escape = 1,
  grave = 41,
  lbracket = 26,
  lcontrol = 29,
  lmenu = 56, -- left alt
  lshift = 42,
  minus = 12,
  numlock = 69,
  pause = 119,
  period = 52,
  rbracket = 27,
  rcontrol = 97,
  rmenu = 100, -- right alt
  rshift = 54,
  scroll = 70, -- scroll lock
  semicolon = 39,
  slash = 53,
  space = 57,
  tab = 15,

  up = 103,
  down = 108,
  left = 105,
  right = 106,
  home = 102,
  ["end"] = 107,
  pageUp = 104,
  pageDown = 109,
  insert = 110,
  delete = 111,

  f1 = 59, f2 = 60, f3 = 61, f4 = 62, f5 = 63, f6 = 64, f7 = 65, f8 = 66, f9 = 67, f10 = 68, f11 = 87, f12 = 88,
  f13 = 183, f14 = 184, f15 = 185, f16 = 186, f17 = 187, f18 = 188, f19 = 189, f20 = 190, f21 = 191, f22 = 192,
  f23 = 193, f24 = 194,

  numpad0 = 82, numpad1 = 79, numpad2 = 80, numpad3 = 81, numpad4 = 75,
  numpad5 = 76, numpad6 = 77, numpad7 = 71, numpad8 = 72, numpad9 = 73,
  numpadadd = 78,
  numpaddecimal = 83,
  numpaddiv = 98,
  numpadenter = 96,
  numpadequals = 117,
  numpadmul = 55,
  numpadsub = 74,
}

local names = {}
for name, code in pairs(keyboard.keys) do
  names[code] = name
end
for code, name in pairs(names) do
  keyboard.keys[code] = name
end

function keyboard.isControl(char)
  return type(char) == "number" and (char < 0x20 or (char >= 0x7F and char <= 0x9F))
end

return keyboard
