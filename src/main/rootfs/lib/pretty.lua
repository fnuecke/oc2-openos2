local pretty = {}

local KEYWORDS = {}
for word in ("and break do else elseif end false for function goto if in local nil not or repeat return then true "
    .. "until while"):gmatch("%a+") do
  KEYWORDS[word] = true
end

local function compare(a, b)
  local ta, tb = type(a), type(b)
  if ta ~= tb then
    return ta == "number" or (tb ~= "number" and ta < tb)
  elseif ta == "number" or ta == "string" then
    return a < b
  end
  return tostring(a) < tostring(b)
end

local function key(k)
  if type(k) == "string" and k:find("^[%a_][%w_]*$") and not KEYWORDS[k] then
    return k
  end
  return "[" .. (type(k) == "string" and string.format("%q", k) or tostring(k)) .. "]"
end

local function format(value, indent, seen, out)
  local meta = getmetatable(value)
  if type(value) == "string" then
    out[#out + 1] = string.format("%q", value)
  elseif type(value) ~= "table" or meta and meta.__tostring then
    out[#out + 1] = tostring(value)
  elseif seen[value] then
    out[#out + 1] = "<" .. tostring(value) .. ">"
  elseif next(value) == nil then
    seen[value] = true
    out[#out + 1] = "{}"
  else
    seen[value] = true
    local inner, count, keys = indent .. "  ", 0, {}
    while rawget(value, count + 1) ~= nil do
      count = count + 1
    end
    for k in next, value do
      if not (math.type(k) == "integer" and k >= 1 and k <= count) then
        keys[#keys + 1] = k
      end
    end
    table.sort(keys, compare)
    out[#out + 1] = "{\n"
    for i = 1, count do
      out[#out + 1] = inner
      format(rawget(value, i), inner, seen, out)
      out[#out + 1] = ",\n"
    end
    for _, k in ipairs(keys) do
      out[#out + 1] = inner .. key(k) .. " = "
      format(rawget(value, k), inner, seen, out)
      out[#out + 1] = ",\n"
    end
    out[#out] = "\n"
    out[#out + 1] = indent .. "}"
  end
end

function pretty.format(value)
  local out = {}
  format(value, "", {}, out)
  return table.concat(out)
end

function pretty.print(...)
  local parts = table.pack(...)
  for i = 1, parts.n do
    parts[i] = pretty.format(parts[i])
  end
  print(table.concat(parts, "\t", 1, parts.n))
end

return pretty
