// The `obsidian` module a tier-1 plugin is handed, plus the CommonJS wrapper
// it expects. Everything here is a plain value or a function: nothing draws.
// A plugin that reaches for something outside this surface throws, and the
// host records that as the reason it could not load — the same refusal the
// classification pass would have made, discovered at runtime instead of by
// reading the source.
//
// Rust owns the file system and the parse index; the `__silica_*` globals are
// the way in. Registrations are kept here as plain objects and read out in one
// go by `__silica_take`, so the boundary is crossed once per plugin load.

"use strict";
(function () {

var __silica_registrations = {
  commands: [],
  ribbon: [],
  tabs: [],
  code_blocks: [],
  post_processors: 0,
  failure: null,
};

var __silica_callbacks = {};
var __silica_bound = 0;

function __silica_fail(message) {
  if (__silica_registrations.failure === null) {
    __silica_registrations.failure = String(message);
  }
}

function __silica_bind(fn) {
  var key = "k" + __silica_bound++;
  __silica_callbacks[key] = fn;
  return key;
}

// ---------------------------------------------------------------------------
// Dates. A subset of moment: the formats a note-naming plugin needs, and
// nothing else. Anything past this throws, which is the honest outcome.
var __moment_units = {
  year: 31536000000, years: 31536000000, y: 31536000000,
  month: 2592000000, months: 2592000000, M: 2592000000,
  week: 604800000, weeks: 604800000, w: 604800000,
  day: 86400000, days: 86400000, d: 86400000,
  hour: 3600000, hours: 3600000, h: 3600000,
  minute: 60000, minutes: 60000, m: 60000,
  second: 1000, seconds: 1000, s: 1000,
};

function __moment_pad(n, width) {
  var s = String(n);
  while (s.length < width) s = "0" + s;
  return s;
}

var __moment_names = {
  months: ["January", "February", "March", "April", "May", "June", "July",
           "August", "September", "October", "November", "December"],
  days: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
};

function moment(value) {
  var date = value === undefined ? new Date()
           : (value instanceof Date ? value : new Date(value));
  if (isNaN(date.getTime())) throw new Error("moment: invalid date");

  var api = {
    _d: date,
    format: function (fmt) {
      var d = api._d;
      var out = "";
      for (var i = 0; i < fmt.length; i++) {
        var c = fmt[i];
        if (c === "[") {
          var close = fmt.indexOf("]", i);
          if (close < 0) throw new Error("moment: unclosed [ in format");
          out += fmt.slice(i + 1, close);
          i = close;
          continue;
        }
        var rest = fmt.slice(i);
        if (rest.startsWith("YYYY")) { out += d.getFullYear(); i += 3; continue; }
        if (rest.startsWith("MMMM")) { out += __moment_names.months[d.getMonth()]; i += 3; continue; }
        if (rest.startsWith("MMM")) { out += __moment_names.months[d.getMonth()].slice(0, 3); i += 2; continue; }
        if (rest.startsWith("dddd")) { out += __moment_names.days[d.getDay()]; i += 3; continue; }
        if (rest.startsWith("ddd")) { out += __moment_names.days[d.getDay()].slice(0, 3); i += 2; continue; }
        if (rest.startsWith("MM")) { out += __moment_pad(d.getMonth() + 1, 2); i += 1; continue; }
        if (rest.startsWith("DD")) { out += __moment_pad(d.getDate(), 2); i += 1; continue; }
        if (rest.startsWith("HH")) { out += __moment_pad(d.getHours(), 2); i += 1; continue; }
        if (rest.startsWith("mm")) { out += __moment_pad(d.getMinutes(), 2); i += 1; continue; }
        if (rest.startsWith("ss")) { out += __moment_pad(d.getSeconds(), 2); i += 1; continue; }
        if (c === "M") { out += d.getMonth() + 1; continue; }
        if (c === "D") { out += d.getDate(); continue; }
        if (c === "H") { out += d.getHours(); continue; }
        if (c === "m") { out += d.getMinutes(); continue; }
        if (c === "s") { out += d.getSeconds(); continue; }
        out += c;
      }
      return out;
    },
    add: function (n, unit) { api._d = new Date(api._d.getTime() + n * __silica_unit(unit)); return api; },
    subtract: function (n, unit) { return api.add(-n, unit); },
    valueOf: function () { return api._d.getTime(); },
    toDate: function () { return api._d; },
    toISOString: function () { return api._d.toISOString(); },
    clone: function () { return moment(new Date(api._d.getTime())); },
  };
  return api;
}

function __silica_unit(unit) {
  if (!(unit in __moment_units)) throw new Error("moment: unsupported unit " + unit);
  return __moment_units[unit];
}

moment.locale = function () { return moment; };
moment.duration = function () { throw new Error("moment.duration is not supported"); };

// ---------------------------------------------------------------------------
// The elements a setting tab or a view would build. There is no DOM: a heading
// or a paragraph of text is recorded so the tab can be drawn natively, and
// anything that would draw — a table, an SVG, a canvas — throws, which marks
// the tab as one this host cannot serve.
function __silica_node(kind, text) {
  var node = {
    tag: kind,
    textContent: text || "",
    children: [],
    empty: function () { node.children.length = 0; node.textContent = ""; return node; },
    createEl: function (tag, opts) {
      if (["h1", "h2", "h3", "h4", "h5", "h6"].indexOf(String(tag)) >= 0) {
        __silica_text("heading", (opts && opts.text) || "");
      } else {
        __silica_text("text", (opts && opts.text) || "");
      }
      return __silica_node(String(tag), (opts && opts.text) || "");
    },
    createDiv: function (opts) { return node.createEl("div", opts); },
    createSpan: function (opts) { return node.createEl("span", opts); },
    appendChild: function (child) { node.children.push(child); return child; },
    addClass: function () { return node; },
    removeClass: function () { return node; },
    setAttr: function () { return node; },
    setText: function (t) { node.textContent = String(t); return node; },
    setCssProps: function () { return node; },
  };
  return node;
}

function __silica_text(kind, text) {
  var tab = __silica_current_tab();
  if (tab) tab.rows.push({ kind: kind, name: String(text || ""), desc: "", control: "none" });
}

var __silica_tab = null;

function __silica_current_tab() { return __silica_tab; }

function __silica_row(control) {
  var tab = __silica_current_tab();
  var row = { kind: "setting", name: "", desc: "", control: control, value: null, options: [] };
  if (tab) tab.rows.push(row);
  return row;
}

// A `Setting` records a row instead of building one. The setters return
// `this`, the callbacks are kept so the native row can call back in, and
// `onChange` is bound the same way the app binds its own rows.
function Setting(containerEl) {
  var row = __silica_row("none");
  var key = null;
  var api = {
    setName: function (n) { row.name = String(n); return api; },
    setDesc: function (d) { row.desc = String(d); return api; },
    setHeading: function () { row.kind = "heading"; row.control = "none"; return api; },
    setClass: function () { return api; },
    setTooltip: function () { return api; },
    setDisabled: function (v) { row.disabled = !!v; return api; },
    addToggle: function (cb) {
      row.control = "toggle"; row.value = false;
      if (cb) cb(__silica_component(row));
      return api;
    },
    addText: function (cb) {
      row.control = "text"; row.value = "";
      if (cb) cb(__silica_component(row));
      return api;
    },
    addTextArea: function (cb) {
      row.control = "text"; row.value = "";
      if (cb) cb(__silica_component(row));
      return api;
    },
    addDropdown: function (cb) {
      row.control = "dropdown"; row.value = "";
      if (cb) cb(__silica_component(row));
      return api;
    },
    addButton: function (cb) {
      row.control = "button"; row.value = "";
      if (cb) cb(__silica_component(row));
      return api;
    },
    addExtraButton: function (cb) {
      row.control = "button"; row.value = "";
      if (cb) cb(__silica_component(row));
      return api;
    },
    then: function (cb) { cb(api); return api; },
  };
  return api;
}

// What a `addToggle` / `addText` / `addDropdown` / `addButton` callback is
// handed: the component the plugin configures. Values land on the recorded row
// and every change is bound, so the native row can drive it.
function __silica_component(row) {
  var api = {
    _key: null,
    getValue: function () { return row.value; },
    setValue: function (v) {
      row.value = v;
      if (api._key !== null) __silica_callbacks[api._key](v);
      return api;
    },
    setPlaceholder: function (p) { row.placeholder = String(p); return api; },
    setLimits: function () { return api; },
    addOption: function (value, label) {
      row.options.push({ value: String(value), label: String(label) });
      if (row.value === "") row.value = String(value);
      return api;
    },
    addOptions: function (options) {
      for (var k in options) api.addOption(k, options[k]);
      return api;
    },
    setButtonText: function (t) { row.value = String(t); return api; },
    setCta: function () { return api; },
    setIcon: function () { return api; },
    setTooltip: function () { return api; },
    setDisabled: function (v) { row.disabled = !!v; return api; },
    onClick: function (fn) { api._key = __silica_bind(fn); return api; },
    onChange: function (fn) { api._key = __silica_bind(fn); return api; },
    then: function (fn) { fn(api); return api; },
  };
  return api;
}

function PluginSettingTab(app, plugin) {
  this.app = app;
  this.plugin = plugin;
  this.containerEl = __silica_node("div", "");
  this.name = plugin && plugin.manifest ? plugin.manifest.name : "";
  this.id = "";
}
PluginSettingTab.prototype.display = function () {};
PluginSettingTab.prototype.hide = function () {};

// ---------------------------------------------------------------------------
// The plugin base class. Every method here registers something; none of them
// draws, which is what makes a plugin written against it runnable here.
function Plugin(app, manifest) {
  this.app = app;
  this.manifest = manifest;
  this._data = null;
  this._data_loaded = false;
}

Plugin.prototype.addCommand = function (command) {
  if (!command || !command.id) {
    __silica_fail("addCommand was given a command with no id");
    return;
  }
  var key = command.callback ? __silica_bind(command.callback) : null;
  // Obsidian knows a command as `pluginId:commandId` and labels it with the
  // plugin's name, which is what keeps two plugins' "Open" apart.
  var id = this.manifest && this.manifest.id ? String(this.manifest.id) : "";
  var name = String(command.name || command.id);
  if (id) {
    name = (this.manifest.name || id) + ": " + name;
  }
  __silica_registrations.commands.push({
    id: id ? id + ":" + String(command.id) : String(command.id),
    name: name,
    callback: key,
  });
};

Plugin.prototype.addRibbonIcon = function (icon, title, callback) {
  var key = callback ? __silica_bind(callback) : null;
  __silica_registrations.ribbon.push({
    icon: String(icon),
    title: String(title),
    callback: key,
  });
  return __silica_node("div", "");
};

Plugin.prototype.addSettingTab = function (tab) {
  if (!tab || typeof tab.display !== "function") {
    __silica_fail("addSettingTab was given something that is not a PluginSettingTab");
    return;
  }
  var record = {
    name: String(tab.name || ""),
    id: String(tab.id || ""),
    rows: [],
  };
  __silica_registrations.tabs.push(record);
  __silica_tab = record;
  try {
    tab.display();
  } catch (error) {
    __silica_fail("its settings tab needs the DOM: " + __silica_message(error));
  } finally {
    __silica_tab = null;
  }
};

Plugin.prototype.registerMarkdownPostProcessor = function (fn, order) {
  if (typeof fn !== "function") {
    __silica_fail("registerMarkdownPostProcessor was given something that is not a function");
    return;
  }
  __silica_registrations.post_processors += 1;
  var key = __silica_bind(fn);
  __silica_raw_post_processor(Number(order) || 0, key);
};

Plugin.prototype.registerMarkdownCodeBlockProcessor = function (language, fn, order) {
  if (typeof fn !== "function") {
    __silica_fail("registerMarkdownCodeBlockProcessor was given something that is not a function");
    return;
  }
  var key = __silica_bind(fn);
  var i = __silica_registrations.code_blocks.length;
  __silica_registrations.code_blocks.push({ language: String(language), callback: key, order: Number(order) || 0 });
  return i;
};

// Registration surfaces that exist so a plugin's own code does not fall over,
// but that change nothing here: the app owns its layout, its workspace events
// and its DOM events.
Plugin.prototype.registerEvent = function () {};
Plugin.prototype.registerDomEvent = function () {};
Plugin.prototype.registerInterval = function (id) { return id; };
Plugin.prototype.registerView = function () {};
Plugin.prototype.registerExtensions = function () {};
Plugin.prototype.addStatusBarItem = function () { return __silica_node("div", ""); };

Plugin.prototype.loadData = function () {
  var text = __silica_raw_read_data();
  this._data = text === "" ? null : JSON.parse(text);
  this._data_loaded = true;
  return Promise.resolve(this._data);
};

Plugin.prototype.saveData = function (data) {
  this._data = data;
  __silica_raw_write_data(JSON.stringify(data));
  return Promise.resolve();
};

// ---------------------------------------------------------------------------
// The vault, the parse index and the app. Reads come from the index the
// reader already built; writes go through the same rules the app uses, except
// that a write to the note open in the editor is refused so a plugin cannot
// clobber an unsaved buffer.
function __silica_app(manifest) {
  var vault = {
    getFiles: function () { return __silica_files(); },
    getMarkdownFiles: function () { return __silica_files(); },
    getAbstractFileByPath: function (path) {
      var file = __silica_file(String(path));
      return file === null ? null : __silica_abstract(file);
    },
    read: function (file) { return Promise.resolve(__silica_raw_vault_read(__silica_path(file))); },
    cachedRead: function (file) { return Promise.resolve(__silica_raw_vault_read(__silica_path(file))); },
    readBinary: function (file) { throw new Error("vault.readBinary is not supported by Silica"); },
    create: function (path, data) { return Promise.resolve(__silica_raw_vault_create(String(path), String(data === undefined ? "" : data))); },
    createFolder: function (path) { __silica_raw_vault_create_folder(String(path)); return Promise.resolve(); },
    modify: function (file, data) { return Promise.resolve(__silica_raw_vault_write(__silica_path(file), String(data))); },
    append: function (file, data) { return Promise.resolve(__silica_raw_vault_append(__silica_path(file), String(data))); },
    delete: function (file) { return Promise.resolve(__silica_raw_vault_delete(__silica_path(file))); },
    rename: function (file, newPath) { throw new Error("vault.rename is not supported; use app.fileManager.renameFile"); },
    trash: function (file) { return Promise.resolve(__silica_raw_vault_delete(__silica_path(file))); },
    getConfig: function () { throw new Error("vault.getConfig is not supported by Silica"); },
    setConfig: function () { throw new Error("vault.setConfig is not supported by Silica"); },
    on: function () { return { unload: function () {} }; },
  };

  var metadataCache = {
    getFileCache: function (file) {
      var text = __silica_raw_meta_cache(__silica_path(file));
      return text === "" ? null : JSON.parse(text);
    },
    getCache: function (path) {
      var text = __silica_raw_meta_cache(String(path));
      return text === "" ? null : JSON.parse(text);
    },
    getFirstLinkpathDest: function (linkpath) {
      var path = __silica_resolve(String(linkpath));
      return path === "" ? null : vault.getAbstractFileByPath(path);
    },
    getTags: function () { throw new Error("metadataCache.getTags is not supported by Silica"); },
    fileToLinktext: function (file) {
      var f = __silica_file(__silica_path(file));
      return f === null ? "" : f.basename;
    },
    resolvedLinks: {},
    unresolvedLinks: {},
    getBacklinksForFile: function () { return { data: {} }; },
    on: function () { return { unload: function () {} }; },
  };

  var commands = {
    commands: {},
    addCommand: function (command) {
      if (!command || !command.id) throw new Error("addCommand was given a command with no id");
      var key = command.callback ? __silica_bind(command.callback) : null;
      __silica_registrations.commands.push({
        id: String(command.id),
        name: String(command.name || command.id),
        callback: key,
      });
      commands.commands[String(command.id)] = command;
      return command;
    },
    removeCommand: function (id) { delete commands.commands[String(id)]; },
    executeCommandById: function (id) {
      var known = commands.commands[String(id)];
      if (known && typeof known.callback === "function") { known.callback(); return true; }
      return __silica_raw_execute(String(id));
    },
    findCommand: function (id) { return commands.commands[String(id)] || null; },
  };

  var fileManager = {
    renameFile: function (file, newPath) { return Promise.resolve(__silica_raw_vault_rename(__silica_path(file), String(newPath))); },
    trashFile: function (file) { return Promise.resolve(__silica_raw_vault_delete(__silica_path(file))); },
    generateMarkdownLink: function (file, sourcePath, subpath, alias) {
      return __silica_markdown_link(__silica_path(file), String(sourcePath || ""), subpath || "", alias || "");
    },
    processFrontMatter: function (file, fn) {
      var path = __silica_path(file);
      var text = __silica_raw_vault_read(path);
      var front = __silica_front_matter(text);
      var changed = { changed: false };
      fn(front, changed);
      __silica_raw_vault_write(path, __silica_with_front_matter(text, JSON.stringify(front)));
      return Promise.resolve();
    },
  };

  var workspace = {
    onLayoutReady: function (fn) { __silica_delay(fn); },
    on: function () { return { unload: function () {} }; },
    getActiveFile: function () {
      var path = __silica_raw_active_file();
      return path === "" ? null : vault.getAbstractFileByPath(path);
    },
    getLeavesOfType: function () { return []; },
    getLeaf: function () { return { openFile: function () { throw new Error("opening a file is not supported by Silica"); } }; },
    getActiveViewOfType: function () { return null; },
  };

  return {
    appId: "silica",
    vault: vault,
    metadataCache: metadataCache,
    commands: commands,
    fileManager: fileManager,
    workspace: workspace,
    lastEvent: null,
    keymap: {},
    scope: { register: function () {} },
  };
}

function __silica_path(file) {
  if (file === null || file === undefined) throw new Error("a file was expected");
  if (typeof file === "string") return file;
  if (typeof file.path === "string") return file.path;
  throw new Error("a file was expected");
}

function __silica_message(error) {
  if (error && error.message) return String(error.message);
  return String(error);
}

// ---------------------------------------------------------------------------
// The boundary. Rust answers with JSON text and does the file system work; the
// shapes a plugin sees are built here.
function __silica_files() { return JSON.parse(__silica_raw_files()); }

function __silica_file(path) {
  var text = __silica_raw_file(String(path));
  return text === "" ? null : JSON.parse(text);
}

function __silica_abstract(file) {
  return {
    path: file.path, name: file.name, extension: file.extension,
    basename: file.basename, parent: null,
    stat: { size: file.size, mtime: file.mtime, ctime: file.mtime },
  };
}

// Obsidian resolves a link by the shortest path ending in it. The vault here
// is small enough that walking the file list is the honest implementation.
function __silica_resolve(linkpath) {
  var wanted = linkpath.split("#")[0].split("|")[0].trim();
  if (wanted === "") return "";
  var files = __silica_files();
  var target = wanted.toLowerCase();
  var withExtension = target.indexOf(".") >= 0 ? target : target + ".md";
  for (var i = 0; i < files.length; i++) {
    if (files[i].path.toLowerCase() === withExtension) return files[i].path;
    if (files[i].basename.toLowerCase() === target) return files[i].path;
  }
  for (var j = 0; j < files.length; j++) {
    if (files[j].path.toLowerCase().endsWith("/" + withExtension)) return files[j].path;
  }
  return "";
}

function __silica_markdown_link(path, sourcePath, subpath, alias) {
  var file = __silica_file(path);
  if (file === null) return "";
  var inner = file.basename + (subpath ? "#" + String(subpath) : "");
  if (alias) return "[[" + inner + "|" + String(alias) + "]]";
  return "[[" + inner + "]]";
}

function __silica_front_matter(text) {
  if (!text.startsWith("---")) return {};
  var end = text.indexOf("\n---", 3);
  if (end < 0) return {};
  var block = text.slice(text.indexOf("\n", 3) + 1, end);
  var out = {};
  var lines = block.split("\n");
  for (var i = 0; i < lines.length; i++) {
    var line = lines[i];
    if (line.trim() === "" || line.startsWith(" ")) continue;
    var colon = line.indexOf(":");
    if (colon < 0) continue;
    var key = line.slice(0, colon).trim();
    var value = line.slice(colon + 1).trim();
    if (value === "true") out[key] = true;
    else if (value === "false") out[key] = false;
    else if (value !== "" && !isNaN(Number(value))) out[key] = Number(value);
    else if (value.startsWith("[") && value.endsWith("]")) {
      out[key] = value.slice(1, -1).split(",").map(function (v) { return v.trim(); })
        .filter(function (v) { return v !== ""; });
    } else if (value === "") out[key] = null;
    else out[key] = value.replace(/^["\']|["\']$/g, "");
  }
  return out;
}

function __silica_with_front_matter(text, json) {
  var body = text.startsWith("---") ? text.slice(Math.max(0, text.indexOf("\n---", 3))) : text;
  if (body.startsWith("\n---")) body = body.slice(4).replace(/^\n/, "");
  var values = JSON.parse(json);
  var block = "---\n";
  for (var key in values) {
    var value = values[key];
    if (Array.isArray(value)) block += key + ": [" + value.join(", ") + "]\n";
    else if (value === null) block += key + ":\n";
    else block += key + ": " + value + "\n";
  }
  return block + "---\n" + body;
}

var __silica_deferred = [];

function __silica_delay(fn) { __silica_deferred.push(fn); }

// ---------------------------------------------------------------------------
// CommonJS, as a bundled `main.js` expects to find it.
var exports = {};
var module = { exports: exports };

function require(name) {
  if (name === "obsidian") return __silica_obsidian;
  throw new Error("Silica cannot require the module '" + name + "'");
}
require.resolve = function (name) { return name; };

var __silica_obsidian = {
  Plugin: Plugin,
  PluginSettingTab: PluginSettingTab,
  Setting: Setting,
  Notice: function Notice(message) { __silica_raw_notice(String(message)); },
  moment: moment,
  Platform: { isDesktopApp: true, isMobileApp: false, isMobile: false, isDesktop: true, isWin: false, isMacOS: false, isLinux: true },
  normalizePath: function (path) {
    var out = String(path).replace(/\\/g, "/").replace(/\/+/g, "/");
    if (out.length > 1) out = out.replace(/\/$/, "");
    return out === "" ? "/" : out;
  },
  requestUrl: function () {
    throw new Error("requestUrl is not supported by Silica; the compatibility host makes no network calls");
  },
  setIcon: function () {},
  getIcon: function () { return null; },
  addIcon: function () {},
  debounce: function (fn, timeout) {
    var handle = null;
    var wait = timeout || 0;
    return function () {
      var args = arguments;
      if (handle !== null) clearTimeout(handle);
      handle = setTimeout(function () { handle = null; fn.apply(null, args); }, wait);
    };
  },
  prepareFuzzySearch: function (query) {
    return function (text) { return String(text).indexOf(String(query)) >= 0 ? { score: 0 } : null; };
  },
  prepareSimpleSearch: function (query) {
    return function (text) { return String(text).indexOf(String(query)) >= 0 ? { score: 0 } : null; };
  },
  htmlToMarkdown: function () { throw new Error("htmlToMarkdown is not supported by Silica"); },
  sanitizeHTMLToDom: function () { throw new Error("sanitizeHTMLToDom is not supported by Silica"); },
  stringifyYaml: function (value) { return JSON.stringify(value); },
  parseYaml: function (text) { return JSON.parse(text); },
  getAllTags: function () { return []; },
  MarkdownRenderer: {},
  Modal: function Modal() { throw new Error("a Modal needs the DOM, which Silica does not have"); },
  ItemView: function ItemView() { throw new Error("an ItemView needs the DOM, which Silica does not have"); },
};

// ---------------------------------------------------------------------------
// Called once, from Rust, after the plugin has loaded: run what the plugin
// deferred to `onLayoutReady`, then hand the registrations back as JSON.
function __silica_take() {
  while (__silica_deferred.length > 0) {
    var fn = __silica_deferred.shift();
    try { fn(); } catch (error) { __silica_fail("onLayoutReady threw: " + __silica_message(error)); }
  }
  return JSON.stringify({
    commands: __silica_registrations.commands,
    ribbon: __silica_registrations.ribbon,
    tabs: __silica_registrations.tabs,
    code_blocks: __silica_registrations.code_blocks,
    post_processors: __silica_registrations.post_processors,
    failure: __silica_registrations.failure,
  });
}

// Run a callback the plugin registered, by the key Rust was given. The value
// is what a setting row changed to; commands ignore it.
function __silica_run(key, value) {
  var fn = __silica_callbacks[key];
  if (typeof fn !== "function") throw new Error("no callback for " + key);
  return fn(value === undefined ? null : value);
}

// Only the host and the plugin's own loader need these in the global scope;
// everything the plugin is meant to see comes through `require`.
globalThis.require = require;
globalThis.module = module;
globalThis.exports = exports;
globalThis.__silica_app = __silica_app;
globalThis.__silica_fail = __silica_fail;
globalThis.__silica_take = __silica_take;
globalThis.__silica_run = __silica_run;
globalThis.__silica_message = __silica_message;
})();
