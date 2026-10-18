-- The Lua side of the `omsi` table: the saved data. Everything else - the functions, the
-- events, timers and watches - is the API registry's (src/api/), bound by src/lua.rs.
local omsi = omsi

-- saved data: omsi.data is written on the way out and read back on the next start
local function dump(v, indent, seen)
  local t = type(v)
  if t == "string" then return string.format("%q", v) end
  if t == "number" then
    if v ~= v then return "0/0" end
    if v == math.huge then return "math.huge" end
    if v == -math.huge then return "-math.huge" end
    return string.format("%.17g", v)
  end
  if t == "boolean" or t == "nil" then return tostring(v) end
  if t ~= "table" then return "nil" end
  if seen[v] then return "nil" end
  seen[v] = true
  local keys = {}
  for k in pairs(v) do
    local kt = type(k)
    if kt == "string" or kt == "number" or kt == "boolean" then keys[#keys + 1] = k end
  end
  table.sort(keys, function(a, b)
    if type(a) == type(b) then return a < b end
    return type(a) < type(b)
  end)
  local inner = indent .. "  "
  local out = { "{\n" }
  for _, k in ipairs(keys) do
    local ks
    if type(k) == "string" and k:match("^[%a_][%w_]*$") then ks = k
    else ks = "[" .. dump(k, inner, seen) .. "]" end
    out[#out + 1] = inner .. ks .. " = " .. dump(v[k], inner, seen) .. ",\n"
  end
  out[#out + 1] = indent .. "}"
  seen[v] = nil
  return table.concat(out)
end

function omsi._save()
  if next(omsi.data) == nil then
    omsi._write_data(nil)
  else
    omsi._write_data("return " .. dump(omsi.data, "", {}) .. "\n")
  end
end

function omsi.save() omsi._save() end

do
  local text = omsi._read_data()
  omsi.data = {}
  if text then
    local chunk = load(text, "=saved data", "t", { math = { huge = math.huge } })
    local ok, t = pcall(chunk or error)
    if ok and type(t) == "table" then omsi.data = t end
  end
end

-- A native process can take seconds: only this plugin's Lua continuation sleeps, never
-- a Rust API call or the game thread. Every resume enters with a fresh frame's API context.
do
  local poll = omsi.process.exit_code
  local create_thread = omsi._budgeted_thread
  omsi._budgeted_thread = nil
  local active, waiting
  local stopping = false
  local queue = {}
  local marker = {}
  local resume_call

  resume_call = function(thread, ...)
    local previous = active
    active = thread
    local result = table.pack(coroutine.resume(thread, ...))
    active = previous
    if not result[1] then
      queue = {}
      error(result[2], 0)
    end
    if coroutine.status(thread) == "suspended" then
      if result[2] ~= marker or previous ~= nil then
        queue = {}
        error("only process.wait in a top-level plugin callback may suspend execution", 0)
      end
      waiting = {thread = thread, handle = result[3]}
      return
    end
    return table.unpack(result, 2, result.n)
  end

  function omsi.process.wait(handle)
    local code, state = poll(handle) -- also enforces the process permission
    if state ~= "running" then return code, state end
    if stopping then error("process.wait cannot suspend a stopping plugin", 2) end
    if coroutine.running() ~= active then
      error("process.wait requires managed plugin code, not a manually resumed coroutine", 2)
    end
    return coroutine.yield(marker, handle)
  end

  function omsi._managed_call(fn, ...)
    if waiting then
      if #queue >= 256 then error("too many deferred plugin callbacks", 0) end
      queue[#queue + 1] = {fn = fn, args = table.pack(...)}
      return
    end
    return resume_call(create_thread(fn), ...)
  end

  function omsi._process_waiting() return waiting ~= nil end

  function omsi._resume_process_wait()
    if not waiting then return false end
    local code, state = poll(waiting.handle)
    if state == "running" then return true end
    local task = waiting
    waiting = nil
    resume_call(task.thread, code, state)
    while not waiting and #queue > 0 do
      local next_call = table.remove(queue, 1)
      omsi._managed_call(next_call.fn, table.unpack(next_call.args, 1, next_call.args.n))
    end
    return true -- even completion consumes this plugin's frame; no duplicate on_frame
  end

  function omsi._cancel_process_wait()
    waiting, queue, active = nil, {}, nil
    stopping = true
  end
end
