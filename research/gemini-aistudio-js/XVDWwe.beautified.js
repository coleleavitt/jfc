"use strict";
this.default_MakerSuite = this.default_MakerSuite || {};
(function(_) {
	try {
		_.umd = [_.qm("fadeInOut", [_.um(":enter", [_.sm({ opacity: 0 }), _.rm("200ms 0ms ease-in", _.sm({ opacity: 1 }))]), _.um(":leave", [_.sm({ opacity: 1 }), _.rm("200ms 0ms ease-out", _.sm({ opacity: 0 }))])])];
		var zmd;
		_.vmd = function(a = {}) {
			var b = {
				showPreview: "true",
				showAssistant: "true"
			};
			if (a.resourceKey) {
				b.resourceKey = a.resourceKey;
			}
			if (a.uri) {
				b.uri = a.uri;
			}
			return b;
		};
		_.r3 = function(a) {
			if (!a) return "";
			switch (a.oE()) {
				case 9: return `/apps/${_.Wo(a).getId()}`;
				case 3: return `/apps/drive/${_.So(a).getResourceId()}`;
				case 2: return `/apps/bundled/${_.fj(a, _.Qo, 2, _.Ro).getId()}`;
				case 4:
					var b = _.fj(a, _.To, 4, _.Ro), c = b.Aq();
					let d = _.l(b, 2);
					a = _.l(b, 3);
					b = b.getPath();
					c = [`/apps/github/${c}/${d}`];
					a && (c.push(`/tree/${a}`), b && c.push(`/${b}`));
					return c.join("");
				case 5: return `/apps/gist/${_.fj(a, _.Vo, 5, _.Ro).getId()}`;
				case 6: return "/apps/zip";
				default: return "";
			}
		};
		_.wmd = function(a) {
			var b;
			var c;
			return _.vmd({
				resourceKey: a == null ? undefined : (b = _.So(a)) == null ? undefined : b.En(),
				uri: a == null ? undefined : (c = _.fj(a, _.sCa, 6, _.Ro)) == null ? undefined : c.Gf()
			});
		};
		_.xmd = function(a, b, c) {
			_.x(function* () {
				var d = _.$4a(b.clone(), c != null ? c : !_.Pm(b, 6));
				a.Cm.update((e) => {
					if (!e) return [d];
					var f = _.Xo(d.Tf());
					var g = e.findIndex((k) => _.Xo(k.Tf()) === f);
					return g !== -1 ? (e = [...e], e[g] = d, e) : [...e, d];
				});
				_.sxb(a, d, true);
			});
		};
		ymd = function(a) {
			return _.uj(a, 1, _.oj());
		};
		zmd = class extends _.h {
			constructor(a) {
				super(a);
			}
			getElement(a) {
				return _.at(this, 1, a);
			}
		};
		var Amd = new _.zdb("45762431", _.bd(zmd)("[]"), zmd);
		var Bmd = new _.az("45756631", "");
		var Cmd = new _.az("45756630", "");
		Dmd = new Map([[6, [() => new Date() < new Date("2024-11-15")]], [8, [() => {
			var a = new Date();
			return a >= new Date("2026-03-23T17:00:00Z") && a < new Date("2026-06-21T17:00:00Z");
		}]]]);
		_.s3 = class {
			constructor() {
				this.window = _.m(_.Sn);
				this.A = _.m(_.odb);
				this.F = _.m(_.Op);
				this.U = _.m(_.pG);
				this.X = Dmd;
				this.H = _.M(-1);
				this.I = _.M(false);
				this.mza = this.F.getFlag(Cmd);
				this.AZa = this.F.getFlag(Bmd);
				this.R = [...ymd(this.F.getFlag(Amd))];
				this.ewa = _.W(() => this.A[this.H()]);
				this.Aga = _.W(() => {
					if (this.I()) return false;
					if (this.mza !== "") {
						if (!this.R.length) return true;
						for (let a of this.R) if (this.U.url().startsWith(a)) return true;
					}
					return this.ewa() !== undefined;
				});
			}
		};
		_.s3.prototype.y6 = _.ba(210);
		_.s3.J = function(a) {
			return new (a || _.s3)();
		};
		_.s3.sa = _.Cd({
			token: _.s3,
			factory: _.s3.J,
			wa: "root"
		});
		var Emd;
		Emd = function(a) {
			return _.x(function* () {
				if (a.A === 0) {
					let { promise: b, resolve: c } = Promise.withResolvers();
					a.Fi.push(c);
					return b;
				}
				a.A--;
			});
		};
		_.Fmd = function(a, b, c) {
			return _.x(function* () {
				if (a.A > 0) {
					if (c !== undefined && a.F + a.A > c) throw Error("nc`" + a.F + "`" + a.A + "`" + c);
					a.F += a.A;
					yield new Promise((d) => {
						setTimeout(d, a.A);
					});
				}
				yield Emd(a.H);
				try {
					if ((yield b()).success) {
						a.A = 0, a.F = 0;
					} else {
						a.A = a.A === 0 ? 500 : Math.min(a.A << 1, 6e4);
					}
				} finally {
					a.H.release();
				}
			});
		};
		_.t3 = class {
			constructor() {
				this.Ia = _.m(_.oF);
				this.A = _.m(_.uG);
				this.F = _.m(_.sdb);
				this.A5 = _.vf([_.Bk(this.Ia.theme), _.Bk(this.A.H)]).pipe(_.uf(([a, b]) => {
					a: switch (a) {
						case "dark":
							a = 2;
							break a;
						case "light":
							a = 1;
							break a;
						default: a = 0;
					}
					return Object.assign({}, this.F, { colorScheme: a }, b ? { productData: { contextError: b } } : {});
				}));
			}
		};
		_.t3.J = function(a) {
			return new (a || _.t3)();
		};
		_.t3.sa = _.Cd({
			token: _.t3,
			factory: _.t3.J,
			wa: "root"
		});
		var Gmd = new _.he("xapFeedbackConfig");
		var Hmd = function(a, b, c) {
			_.IBb(a, b, c);
		};
		var u3 = class {};
		u3.J = function(a) {
			return new (a || u3)();
		};
		u3.sa = _.Cd({
			token: u3,
			factory: u3.J,
			wa: "root"
		});
		Imd = function(a) {
			var b;
			var c = (b = a.TD()) != null ? b : a.config;
			b = Object.assign({}, c);
			c = b.productData;
			var d = b.window;
			delete b.window;
			delete b.productData;
			Hmd(Object.assign({}, b, {
				callback: (e) => {
					a.SZa.emit({ y7b: e });
				},
				onLoadCallback: () => {
					if (a.element.nativeElement.closest("[popover]")) {
						var e = a.document.getElementById("google-feedback");
						if (e && "showPopover" in e) {
							e.style.background = "none", e.style.border = "none", e.style.padding = "0", e.setAttribute("popover", "manual"), e.hidePopover(), e.showPopover();
						}
					}
					a.TZa.emit();
				}
			}), c, d);
		};
		_.v3 = class {
			constructor() {
				this.feedback = _.m(u3);
				this.element = _.m(_.Jf);
				this.document = _.m(_.Xk);
				this.config = _.m(Gmd, { optional: true });
				this.TD = _.Li(undefined, Object.assign({}, {}, { alias: "xapFeedbackConfig" }));
				this.SZa = _.Ki();
				this.TZa = _.Ki();
			}
		};
		_.v3.J = function(a) {
			return new (a || _.v3)();
		};
		_.v3.Oa = _.We({
			type: _.v3,
			da: [[
				"button",
				"xapFeedback",
				""
			]],
			Ja: function(a, b) {
				if (a & 1) {
					_.J("click", function() {
						return Imd(b);
					});
				}
			},
			inputs: { TD: [
				1,
				"xapFeedbackConfig",
				"feedbackConfig"
			] },
			outputs: {
				SZa: "xapFeedbackCompleted",
				TZa: "xapFeedbackLoaded"
			}
		});
		var Jmd;
		_.w3 = function(a) {
			return a.startsWith("models/gemma-");
		};
		Jmd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.Kmd = function(a, b) {
			a.I = b != null ? b : a.F().map((c) => c.getValues());
		};
		var Omd;
		var Pmd;
		_.Lmd = function(a, b = false) {
			var c = b ? {
				ctrl: "⌃",
				alt: "⌥",
				shift: "⇧",
				meta: "⌘",
				cmd: "⌘",
				command: "⌘"
			} : _.Baa() ? {
				ctrl: "Ctrl",
				alt: "Alt",
				shift: "Shift",
				meta: "Super"
			} : {
				ctrl: "Ctrl",
				alt: "Alt",
				shift: "Shift",
				meta: "Win"
			};
			return a.map((d) => c[d.toLowerCase()] || d.toUpperCase());
		};
		Nmd = function(a, b) {
			for (let c of a.A().values()) if (a.F().has(c.id) && Mmd(b, c)) return c.id;
			return null;
		};
		Omd = function(a, b) {
			var c = a.A().get(b);
			if (!c) return [];
			var d = new Set();
			for (let e of a.A().values()) e.id !== b && a.F().has(e.id) && Mmd(c, e) && d.add(e.id);
			return Array.from(d);
		};
		Pmd = function(a, b, c) {
			a.I.set(b, c);
		};
		Qmd = function(a, b, c) {
			b = b.map((f) => f.toLowerCase());
			var d = new Set();
			if (a.ctrlKey) {
				d.add("ctrl");
			}
			if (a.altKey) {
				d.add("alt");
			}
			if (a.shiftKey) {
				d.add("shift");
			}
			if (a.metaKey) {
				d.add("meta");
			}
			var e = a.key.toLowerCase();
			a = a.code;
			if (![
				"ctrl",
				"alt",
				"shift",
				"meta"
			].includes(e)) {
				a === "Slash" ? d.add("/") : a.startsWith("Key") ? d.add(a.substring(3).toLowerCase()) : a.startsWith("Digit") ? d.add(a.substring(5)) : d.add(e);
			}
			if (d.size !== b.length) return false;
			for (let f of b) if (!d.has(c ? f.replace("cmd", "meta").replace("command", "meta") : f)) return false;
			return true;
		};
		Mmd = function(a, b) {
			var c = _.Aa();
			a = c && a.St ? a.St : a.keys;
			b = c && b.St ? b.St : b.keys;
			if (a.length !== b.length) return false;
			c = new Set(a.map((d) => d.toLowerCase()));
			for (let d of b) if (!c.has(d.toLowerCase())) return false;
			return true;
		};
		_.x3 = class {
			constructor() {
				this.document = _.m(_.Xk);
				this.ub = _.m(_.ag);
				this.dialog = _.m(_.rC);
				this.veLoggingService = _.m(_.Ry);
				this.A = _.M(new Map());
				this.groups = new Map();
				this.F = _.M(new Set());
				this.R = new Set();
				this.H = new Map();
				this.I = new Map();
				_.W(() => {
					var a = this.A();
					var b = this.F();
					return Array.from(b).map((c) => a.get(c)).filter((c) => !!c);
				});
			}
			register(a, b) {
				var c = _.m(_.ag);
				var d = _.Aa();
				a.D9 = _.Lmd(d && a.St ? a.St : a.keys, d);
				if (this.A().has(a.id)) {
					console.warn(`Shortcut with ID "${a.id}" already exists. Replacing.`), this.unregister(a.id);
				}
				if (b && !this.groups.has(b)) throw Error("ni`" + b);
				if (d = Nmd(this, a)) throw Error("oi`" + d);
				this.A.update((e) => new Map(e).set(a.id, a));
				d = true;
				if (b) {
					let e = this.groups.get(b);
					e.f$a.push(a);
					this.H.set(a.id, b);
					if (!e.active) {
						d = false;
					}
				}
				if (d) {
					this.F.update((e) => new Set(e).add(a.id));
				} else {
					this.F.update((e) => {
						e = new Set(e);
						e.delete(a.id);
						return e;
					});
				}
				if (c) {
					c.Hc(() => {
						this.unregister(a.id);
					});
				}
			}
			unregister(a) {
				if (this.A().has(a)) {
					this.A.update((c) => {
						c = new Map(c);
						c.delete(a);
						return c;
					});
					this.F.update((c) => {
						c = new Set(c);
						c.delete(a);
						return c;
					});
					var b = this.H.get(a);
					if (b) {
						if (b = this.groups.get(b)) b.f$a = b.f$a.filter((c) => c.id !== a);
						this.H.delete(a);
					}
				}
			}
			iha() {
				this.A.set(new Map());
				this.groups.clear();
				this.F.set(new Set());
				this.R.clear();
				this.H.clear();
				this.I.clear();
			}
			wK(a) {
				if (!this.A().has(a)) throw Error("pi`" + a);
				var b = Omd(this, a);
				if (b.length > 0) throw Error("qi`" + a + "`" + b.join(", "));
				this.F.update((c) => new Set(c).add(a));
			}
			g6() {
				if (!this.A().has(undefined)) throw Error("ri`undefined");
				this.F.update((a) => {
					a = new Set(a);
					a.delete(undefined);
					return a;
				});
			}
			jha() {
				this.I.clear();
			}
			init() {
				Pmd(this, "ignore-inputs", (a) => (a = a.target) ? !(a.isContentEditable || [
					"INPUT",
					"TEXTAREA",
					"SELECT"
				].includes(a.tagName)) : true);
				Pmd(this, "ignore-dialog-open", () => this.dialog.A.length === 0);
				_.Af(this.document, "keydown").pipe(_.Ak(this.ub)).subscribe((a) => {
					var b = _.Aa();
					for (let v of this.F()) {
						let w = this.A().get(v);
						var c;
						if (c = w) {
							a: {
								c = a;
								for (let D of this.I.values()) if (!D(c)) {
									c = false;
									break a;
								}
								var d = this.H.get(w.id);
								if (d && (d = this.groups.get(d)) && typeof d.filter === "function" && !d.filter(c)) {
									c = false;
									break a;
								}
								c = w.filter && !w.filter(c) ? false : true;
							}
							c = c && Qmd(a, b && w.St ? w.St : w.keys, b);
						}
						if (c) {
							try {
								var e = this.veLoggingService;
								var f = new _.Ky();
								var g = new Jmd();
								var k = _.Lj(g, 1, w.id);
								var p = _.Lj(k, 2, "global");
								var r = _.ln(f, Jmd, 7, p);
								_.Qy(e, 305481, r);
								w.action(a);
								a.preventDefault();
							} catch (D) {
								console.error(`Error executing shortcut ${w.id}:`, D);
							}
							break;
						}
					}
				});
			}
		};
		_.x3.J = function(a) {
			return new (a || _.x3)();
		};
		_.x3.sa = _.Cd({
			token: _.x3,
			factory: _.x3.J,
			wa: "root"
		});
		var Rmd;
		Rmd = function(a) {
			var b = _.my(a);
			return _.Im(a, 26) ? "video_camera_front" : b.includes(15) ? "image_edit_auto" : b.includes(14) ? "video_spark" : "spark";
		};
		Smd = function(a) {
			switch (a) {
				case 1: return 1;
				case 2: return 2;
				case 3: return 3;
				default: return 0;
			}
		};
		_.y3 = function(a, b) {
			return _.cn(a, 1, b);
		};
		_.z3 = class extends _.h {
			constructor(a) {
				super(a);
			}
			sB() {
				return _.yj(this, 5);
			}
		};
		_.Tmd = function(a, b) {
			return _.ln(a, _.z3, 8, b);
		};
		Umd = function(a) {
			var b = _.Aa();
			return Array.from(a.A().values()).filter((c) => !c.d7b).map((c) => Object.assign({}, c, { D9: _.Lmd(b && c.St ? c.St : c.keys, b) }));
		};
		_.A3 = class {
			constructor() {
				this.R = _.m(_.Zq);
				this.F = _.m(_.LK);
				this.H = _.m(_.Cl);
				this.window = _.m(_.Sn);
				this.A = _.m(_.BF);
				this.I = _.m(_.fK);
			}
		};
		_.A3.J = function(a) {
			return new (a || _.A3)();
		};
		_.A3.sa = _.Cd({
			token: _.A3,
			factory: _.A3.J,
			wa: "root"
		});
		var Vmd = {
			title: "Command Palette",
			placeholder: "Start a chat or vibe code an app",
			source: []
		};
		var $md;
		_.Wmd = function(a) {
			a.query.set("");
		};
		_.B3 = function(a, b) {
			a.stack.update((c) => c ? [...c, b] : [b]);
			_.Wmd(a);
		};
		_.Xmd = function(a, b, c) {
			var d = typeof b === "string" ? new Set([b]) : b;
			a.commands.update((e) => e.filter((f) => !d.has(f.id)));
			if (c && a.A.has(c)) {
				let e = a.A.get(c);
				d.forEach((f) => {
					e.items.delete(f);
				});
				if (e.items.size === 0) {
					a.A.delete(c);
				}
			} else c || (a.A.forEach((e) => {
				d.forEach((f) => {
					e.items.delete(f);
				});
			}), a.A.forEach((e) => {
				if (e.items.size === 0) {
					a.A.delete(e.id);
				}
			}));
		};
		Ymd = function(a, b = []) {
			if (a.A.has("root")) {
				console.warn("Command group with ID \"root\" already exists.");
			} else {
				a.A.set("root", {
					id: "root",
					items: new Set()
				}), b.length > 0 && a.registerCommand(b, "root");
			}
		};
		_.Zmd = function(a, b) {
			a.H.navigate(["/prompts/new_chat"], b ? { queryParams: { prompt: b } } : undefined);
		};
		$md = function(a) {
			Ymd(a, [
				{
					id: "root-build",
					label: "/build",
					description: "Vibe code an app",
					icon: "design_services",
					variant: "menu",
					action: () => {
						_.B3(a, {
							title: "Build App",
							xf: {
								label: "/build",
								icon: "design_services"
							},
							placeholder: "Describe your app",
							source: null,
							lv: "Building app",
							wj: (b) => _.x(function* () {
								if (b) {
									yield a.H.navigate(["/apps"], {
										queryParams: { prompt: b },
										cC: "reload"
									});
								}
							})
						});
					}
				},
				{
					id: "root-new-chat",
					label: "/chat",
					description: "Start a chat with a model",
					icon: "chat_spark",
					variant: "menu",
					action: () => {
						var b = {
							title: "New Chat",
							xf: {
								label: "/chat",
								icon: "chat_spark"
							},
							placeholder: "Select a model...",
							source: a.U()
						};
						_.B3(a, b);
					}
				},
				{
					id: "root-keys",
					label: "/key",
					description: "Create an API key",
					icon: "key",
					variant: "menu",
					action: () => {
						_.B3(a, {
							title: "Create API Key",
							xf: {
								label: "/key",
								icon: "key"
							},
							placeholder: "Enter a name for your API key...",
							source: null,
							Cwb: true,
							wj: (b) => {
								a.dialog.open(_.AE, { data: {
									JW: b,
									Nx: true,
									Hv: true,
									II: true
								} });
							}
						});
					}
				},
				{
					id: "root-shortcuts",
					label: "/shortcuts",
					description: "View keyboard shortcuts",
					icon: "keyboard_return",
					variant: "menu",
					action: () => {
						var b = {
							title: "Keyboard Shortcuts",
							xf: {
								label: "/shortcuts",
								icon: "keyboard_return"
							},
							placeholder: "Search for a shortcut",
							source: Umd(a.F)
						};
						_.B3(a, b);
					}
				}
			]);
		};
		_.C3 = class {
			open(a) {
				if (!this.Tc()) {
					var b;
					if (!(this.stack() && ((b = this.stack()) == null ? undefined : b.length) !== 0)) {
						this.stack.set([Vmd]);
					}
					this.Tc.set(true);
					b = this.veLoggingService;
					var c = new _.Ky();
					var d = _.y3(new _.z3(), this.I() === "overlay" ? 1 : 2);
					a = _.cn(d, 9, Smd(a));
					_.Qy(b, 305484, _.Tmd(c, a));
				}
			}
			close(a = true) {
				this.Tc.set(false);
				if (a) {
					_.Wmd(this);
				}
			}
			registerCommand(a, b) {
				var c = Array.isArray(a) ? a : [a];
				var d = _.m(_.ag);
				if (b && !this.A.has(b)) {
					this.A.set(b, {
						id: b,
						items: new Set()
					});
				}
				this.commands.update((e) => {
					var f = [];
					var g = new Set(e.map((k) => k.id));
					for (let k of c) g.has(k.id) ? console.warn(`Command with ID "${k.id}" already registered.`) : (f.push(k), g.add(k.id), b && this.A.get(b).items.add(k.id));
					return [...e, ...f];
				});
				if (d) {
					d.Hc(() => {
						var e = new Set(c.map((f) => f.id));
						_.Xmd(this, e, b);
					});
				}
			}
			search(a) {
				var b = a.toLowerCase().trim();
				if (!b) return [];
				a = this.X();
				var c = this.stack();
				if (c && c.length > 1) if (c = c[c.length - 1], c.source) {
					let d = new Set(c.source.map((e) => e.id));
					a = a.filter((e) => d.has(e.item.id));
				} else return [];
				return a.map((d) => {
					var e = d.item;
					var f = d.label;
					var g = d.description;
					d = d.category;
					if (f === b) f = 1e3;
					else if (f.startsWith(b)) f = 800;
					else if (f.includes(b)) f = 400;
					else if (d.includes(b)) f = 200;
					else if (g.includes(b)) f = 100;
					else {
						let k = d = g = 0;
						let p = -1;
						for (; d < b.length && g < f.length;) b[d] === f[g] && (k = p !== -1 && g === p + 1 ? k + 15 : g === 0 || f[g - 1] === " " ? k + 10 : k + 5, p = g, d++), g++;
						f = d === b.length ? k : 0;
					}
					return {
						item: e,
						score: f
					};
				}).filter((d) => d.score > 0).sort((d, e) => e.score - d.score || d.item.label.localeCompare(e.item.label)).map((d) => d.item);
			}
			constructor() {
				this.F = _.m(_.x3);
				this.H = _.m(_.Cl);
				this.Tc = _.M(false);
				this.I = _.M("overlay");
				this.query = _.M("");
				this.dialog = _.m(_.rC);
				this.R = _.m(_.BF);
				this.Ia = _.m(_.oF);
				this.veLoggingService = _.m(_.Ry);
				this.stack = _.M();
				this.commands = _.M([]);
				this.A = new Map();
				this.X = _.W(() => {
					var a = [
						...this.commands(),
						...Umd(this.F),
						...this.U()
					];
					var b = new Map();
					for (let c of a) c.Sa || b.has(c.id) || b.set(c.id, {
						item: c,
						label: c.label.toLowerCase(),
						description: (c.description || "").toLowerCase(),
						category: (c.category || "").toLowerCase()
					});
					return Array.from(b.values());
				});
				this.U = _.W(() => {
					var a = this.R.Ch();
					var b = this.Ia.U();
					a = a.filter((d) => {
						var e = b.get(d.getName());
						d = _.my(d).includes(10);
						return e || d;
					}).map((d) => {
						var e = undefined;
						if (!b.get(d.getName()) && _.my(d).includes(10)) {
							e = "Featured";
						}
						return {
							id: `chat-${d.getName().replace("models/", "")}`,
							label: d.getDisplayName(),
							description: `Start a chat with ${d.getDisplayName()}`,
							icon: Rmd(d),
							variant: "menu",
							action: () => {
								_.Wmd(this);
								var f = d.getName().replace("models/", "");
								var g = {
									title: `Chat with ${d.getDisplayName()}`,
									xf: {
										label: d.getDisplayName(),
										icon: Rmd(d)
									},
									placeholder: "Type your prompt...",
									source: null,
									wj: (k) => {
										var p = _.txa(d);
										this.H.navigate([p], { queryParams: {
											model: f,
											prompt: k
										} });
										return Promise.resolve("Playground created");
									}
								};
								_.B3(this, g);
							},
							category: e
						};
					});
					var c = a.some((d) => d.category === "Featured");
					a.push({
						id: "chat-explore-other-models",
						label: "Explore other models in the playground",
						description: "See all available models",
						icon: "chat_spark",
						variant: "navigation",
						action: () => {
							_.Zmd(this, this.query());
						},
						category: "Featured",
						jKa: c
					});
					return a;
				});
				$md(this);
				_.Rpb(this.R);
				this.F.register({
					id: "open-omnibar-shortcuts",
					keys: [
						"ctrl",
						"shift",
						"/"
					],
					St: [
						"meta",
						"shift",
						"/"
					],
					label: "View keyboard shortcuts",
					description: "View a list of keyboard shortcuts",
					Jpa: true,
					action: () => {
						this.open();
						var a = this.stack();
						if (!(a && a.length > 0 && a[a.length - 1].title === "Keyboard Shortcuts")) {
							a = {
								title: "Keyboard Shortcuts",
								xf: {
									label: "/shortcuts",
									icon: "keyboard_return"
								},
								placeholder: "Search for a shortcut",
								source: Umd(this.F)
							}, _.B3(this, a);
						}
					},
					variant: "command"
				});
			}
		};
		_.C3.J = function(a) {
			return new (a || _.C3)();
		};
		_.C3.sa = _.Cd({
			token: _.C3,
			factory: _.C3.J,
			wa: "root"
		});
		var dnd;
		var end;
		and = function(a, b, c) {
			var d = a instanceof _.cl ? a : _.pA(b, a);
			return _.W(() => {
				var e;
				var f;
				return _.Ysa((f = (e = b.fa()) == null ? undefined : e.bR) != null ? f : new _.cl(), d, Object.assign({}, _.Deb, c));
			});
		};
		bnd = function(a) {
			return !!(a.paths || a.h_ || a.queryParams || a.fragment);
		};
		dnd = function(a) {
			var b;
			if (!((b = a.F) == null)) {
				b.unsubscribe();
			}
			b = [...a.links.toArray(), a.link].filter((c) => !!c).map((c) => c.Oma);
			a.F = _.hf(b).pipe(_.xf()).subscribe((c) => {
				if (a.Vl !== cnd(a, a.A)(c)) {
					a.update();
				}
			});
		};
		cnd = function(a, b) {
			var c;
			var d = bnd(a.ooa) ? a.ooa : (c = a.ooa.exact) != null && c ? Object.assign({}, _.Ceb) : Object.assign({}, _.Deb);
			return (e) => {
				if (e = e.Ea) {
					e = and(e, b, d), e = _.Qd(e);
				} else {
					e = false;
				}
				return e;
			};
		};
		end = function(a) {
			var b = cnd(a, a.A);
			return a.link && b(a.link) || a.links.some(b);
		};
		_.D3 = class {
			get isActive() {
				return this.Vl;
			}
			constructor(a, b, c, d) {
				this.A = a;
				this.element = b;
				this.Dc = c;
				this.H = d;
				this.qt = [];
				this.Vl = false;
				this.ooa = { exact: false };
				this.o2a = new _.pm();
				this.link = _.m(_.sA, { optional: true });
				this.I = a.events.subscribe((e) => {
					if (e instanceof _.yl) {
						this.update();
					}
				});
			}
			Oj() {
				_.mf(this.links.changes, _.mf(null)).pipe(_.xf()).subscribe(() => {
					this.update();
					dnd(this);
				});
			}
			set nOb(a) {
				this.qt = (Array.isArray(a) ? a : a.split(" ")).filter((b) => !!b);
			}
			Wb() {
				this.update();
			}
			Ba() {
				this.I.unsubscribe();
				var a;
				if (!((a = this.F) == null)) {
					a.unsubscribe();
				}
			}
			update() {
				if (this.links && this.A.H) {
					queueMicrotask(() => {
						var a = end(this);
						this.qt.forEach((b) => {
							if (a) {
								this.Dc.Gr(this.element.nativeElement, b);
							} else {
								this.Dc.Cx(this.element.nativeElement, b);
							}
						});
						if (a && this.zUa !== undefined) {
							this.Dc.setAttribute(this.element.nativeElement, "aria-current", this.zUa.toString());
						} else {
							this.Dc.removeAttribute(this.element.nativeElement, "aria-current");
						}
						if (this.Vl !== a) {
							this.Vl = a, this.H.lb(), this.o2a.emit(a);
						}
					});
				}
			}
		};
		_.D3.J = function(a) {
			return new (a || _.D3)(_.Dg(_.Cl), _.Dg(_.Jf), _.Dg(_.cm), _.Dg(_.Hu));
		};
		_.D3.Oa = _.We({
			type: _.D3,
			da: [[
				"",
				"routerLinkActive",
				""
			]],
			Ud: function(a, b, c) {
				if (a & 1) {
					_.bi(c, _.sA, 5);
				}
				if (a & 2) {
					let d;
					if (_.ei(d = _.fi())) {
						b.links = d;
					}
				}
			},
			inputs: {
				ooa: "routerLinkActiveOptions",
				zUa: "ariaCurrentWhenActive",
				nOb: "routerLinkActive"
			},
			outputs: { o2a: "isActiveChange" },
			Cc: ["routerLinkActive"],
			features: [_.su]
		});
		_.E3 = {
			class: "active",
			options: {
				paths: "subset",
				queryParams: "subset",
				fragment: "ignored",
				h_: "ignored"
			}
		};
		var fnd;
		var hnd;
		var K3;
		var tnd;
		fnd = function(...a) {
			var b = _.kf(a);
			var c = _.Xha(a);
			return c.length ? new _.ef((d) => {
				var e = c.map(() => []);
				var f = c.map(() => false);
				d.add(() => {
					e = f = null;
				});
				for (let g = 0; !d.closed && g < c.length; g++) _.gf(c[g]).subscribe(new _.sf(d, (k) => {
					e[g].push(k);
					if (e.every((p) => p.length)) {
						k = e.map((p) => p.shift()), d.next(b ? b(...k) : k), e.some((p, r) => !p.length && f[r]) && d.complete();
					}
				}, undefined, () => {
					f[g] = true;
					if (!e[g].length) {
						d.complete();
					}
				}));
				return () => {
					e = f = null;
				};
			}) : _.Ef;
		};
		gnd = function(a) {
			return a ? _.Pla(() => new _.Wg(), a) : _.Pla(new _.Wg());
		};
		hnd = function(a) {
			var b = a.ub;
			return new _.ef((c) => {
				var d = b == null ? undefined : b.Hc(() => c.complete());
				var e = a.subscribe((f) => c.next(f));
				return () => {
					e.unsubscribe();
					if (!(d == null)) {
						d();
					}
				};
			});
		};
		_.F3 = function(a) {
			return {
				id: "nav_button_tour",
				showGotItButton: false,
				steps: [{
					elementId: "nav-button",
					stepId: "nav-button-step",
					title: "New: Control your API cost",
					html: ind.Em("<span class=\"spend-cap-nav-marker\" style=\"display: none;\"></span>Set a monthly spend cap to prevent accidental overspending"),
					link: {
						label: "Create a spend cap",
						url: a
					}
				}]
			};
		};
		_.G3 = function(a) {
			return {
				id: "spend_nav_item_tour",
				showGotItButton: false,
				steps: [{
					elementId: "spend-nav-item",
					stepId: "spend-nav-item-step",
					title: "New: Control your API cost",
					html: ind.Em("<span class=\"spend-cap-nav-marker\" style=\"display: none;\"></span>Set a monthly spend cap to prevent accidental overspending"),
					link: {
						label: "Create a spend cap",
						url: a
					}
				}]
			};
		};
		_.H3 = function(a) {
			return {
				id: "dashboard_nav_items_tour",
				showGotItButton: false,
				steps: [{
					elementId: "dashboard-nav-item",
					stepId: "dashboard-nav-item-step",
					title: "New: Control your API cost",
					html: ind.Em("<span class=\"spend-cap-nav-marker\" style=\"display: none;\"></span>Set a monthly spend cap to prevent accidental overspending"),
					link: {
						label: "Create a spend cap",
						url: a
					}
				}]
			};
		};
		_.I3 = function() {
			return {
				id: "spend_page_tour",
				showGotItButton: true,
				steps: [{
					elementId: "project-usage-limit-card",
					stepId: "project-usage-limit-card-step",
					title: "New: Control your API cost",
					timeout: 5e3,
					html: ind.Em("<span class=\"spend-cap-project-usage-limit-card-marker\" style=\"display: none;\"></span>Set a monthly spend cap to prevent accidental overspending")
				}, {
					elementId: "project-usage-amount",
					stepId: "project-usage-amount-step",
					title: "Current monthly spend",
					timeout: 5e3,
					html: ind.Em("<span class=\"spend-cap-project-usage-amount-marker\" style=\"display: none;\"></span>Information updates within 10 minutes. You may exceed your cap during this delay.")
				}]
			};
		};
		jnd = function(a) {
			if (a & 1) {
				_.F(0, "h2", 11)(1, "span", 23), _.R(2), _.H()();
			}
			if (a & 2) {
				a = _.K(), _.E("id", a.yya), _.y(2), _.U(a.step().title);
			}
		};
		knd = function(a, b) {
			if (a & 1) {
				_.I(0, "li", 25);
			}
			if (a & 2) {
				a = b.jb, b = _.K(2), _.P("current", a === b.Vp());
			}
		};
		mnd = function(a) {
			if (a & 1) {
				_.F(0, "ol", 17), _.Ah(1, knd, 1, 2, "li", 24, _.yh), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.Bh(_.zi(0, lnd).constructor(a.Yca()));
			}
		};
		ond = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 26, 2);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					nnd(c, 3);
					return _.t();
				})("keydown", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(J3(d, c, "actionButton"));
				});
				_.R(2);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("href", a.linkUrl, _.rg)("buttonType", a.Qea.ACTION)("tourTheme", a.HC), _.y(2), _.S(" ", a.bEa, " ");
			}
		};
		pnd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 27, 3);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.oHa());
				})("keydown", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(J3(d, c, "backButton"));
				});
				_.Mh(2, 6);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("buttonType", a.Qea.BACK)("tourTheme", a.HC), _.wh("aria-controls", a.Apa);
			}
		};
		qnd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 28, 4);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.aN());
				})("keydown", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(J3(d, c, "nextButton"));
				});
				_.Mh(2, 7);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("buttonType", a.Qea.znb)("tourTheme", a.HC), _.wh("aria-controls", a.Apa);
			}
		};
		snd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 29, 5);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(rnd(c));
				})("keydown", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(J3(d, c, "gotItButton"));
				});
				_.Mh(2, 8);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("buttonType", a.Qea.Kkb)("tourTheme", a.HC), _.wh("aria-controls", a.Apa);
			}
		};
		K3 = _.JVa(new _.LVa());
		tnd = new Map(K3.A.R);
		tnd.set("style", { Np: 4 });
		K3.A = new _.Gv(K3.A.H, K3.A.A, K3.A.I, tnd, K3.A.F);
		var ind = K3.build();
		var L3 = class {
			constructor() {
				this.A = new Map();
			}
			add(a, b, c = false) {
				if (this.A.has(a)) {
					if (!c) {
						this.A.get(a).next(b);
					}
				} else {
					c = new _.Zg(1), this.A.set(a, c), c.next(b);
				}
			}
			observe(a) {
				if (!this.A.has(a)) {
					this.A.set(a, new _.Zg(1));
				}
				return this.A.get(a).asObservable();
			}
			remove(a) {
				var b;
				if (!((b = this.A.get(a)) == null)) {
					b.complete();
				}
				this.A.delete(a);
			}
		};
		L3.J = function(a) {
			return new (a || L3)();
		};
		L3.sa = _.Cd({
			token: L3,
			factory: L3.J,
			wa: "root"
		});
		var vnd = { HC: "LEGACY" };
		var wnd = new _.he("STEP_OVERLAY_CONFIG_TOKEN");
		var znd = class {
			constructor() {
				this.HC = _.V("LEGACY");
				this.buttonType = _.Li.required();
				this.lVa = ynd;
				this.format = {
					format: 0,
					color: undefined
				};
				this.A = new Map([
					["REACH_PRIMARY", new Map([
						[0, {
							format: 1,
							color: "primary"
						}],
						[1, {
							format: 1,
							color: "primary"
						}],
						[2, {
							format: 1,
							color: undefined
						}],
						[3, {
							format: 1,
							color: undefined
						}]
					])],
					["REACH_WHITE", new Map([
						[0, {
							format: 0,
							color: "primary"
						}],
						[1, {
							format: 0,
							color: "primary"
						}],
						[2, {
							format: 1,
							color: "primary"
						}],
						[3, {
							format: 1,
							color: "primary"
						}]
					])],
					["LEGACY", new Map([
						[0, {
							format: 0,
							color: undefined
						}],
						[1, {
							format: 0,
							color: undefined
						}],
						[2, {
							format: 1,
							color: "primary"
						}],
						[3, {
							format: 0,
							color: undefined
						}]
					])]
				]);
			}
			ib() {
				this.format = this.A.get(this.HC()).get(this.buttonType());
			}
		};
		znd.J = function(a) {
			return new (a || znd)();
		};
		znd.Oa = _.We({
			type: znd,
			da: [[
				"button",
				"mat-button",
				"",
				"buttonFormat",
				""
			], [
				"a",
				"mat-button",
				"",
				"buttonFormat",
				""
			]],
			Ua: 9,
			Ja: function(a, b) {
				if (a & 2) {
					_.wh("color", b.format.color), _.P("mdc-button--unelevated", b.format.format === b.lVa.JOa)("mat-mdc-unelevated-button", b.format.format === b.lVa.JOa)("mat-primary", b.format.color === "primary")("mat-unthemed", b.format.color !== "primary");
				}
			},
			inputs: {
				HC: [1, "tourTheme"],
				buttonType: [1, "buttonType"]
			}
		});
		var lnd = () => [];
		var Gnd = class {
			constructor(a, b) {
				this.Ga = a;
				if (this.A = b) this.A.F = this;
			}
			get next() {
				var a = this.F;
				return (a == null ? 0 : a.Ga) ? a : a == null ? undefined : a.next;
			}
			get previous() {
				var a = this.A;
				return (a == null ? 0 : a.Ga) ? a : a == null ? undefined : a.previous;
			}
		};
		var nnd = function(a, b) {
			if (a.stepId && a.onNavigation()) {
				a.onNavigation()(b, a.stepId);
			}
		};
		var Hnd = class {
			constructor() {
				this.el = _.m(_.Jf);
				this.DK = _.m(_.im, { optional: true }) === "NoopAnimations";
				this.step = _.Li.required();
				this.Vp = _.Li.required();
				this.Yca = _.Li.required();
				this.onNavigation = _.V();
				this.showGotItButton = _.V();
				this.navigation = _.Ki();
				this.exit = _.Ki();
				this.FA = [];
				this.yya = "";
				this.bEa = "Learn More";
				this.xQa = und;
				var a;
				this.F = (a = _.m(wnd, { optional: true })) != null ? a : vnd;
				this.HC = this.F.HC;
				this.Qea = xnd;
				this.A = new Map([]);
				this.Ey = _.Ni.required("contentContainer", { read: _.Jf });
				this.pWa = _.Ni.required("closeButton", { read: _.Jf });
				this.rTa = _.Ni("actionButton", Object.assign({}, {}, { read: _.Jf }));
				this.L5a = _.Ni("nextButton", Object.assign({}, {}, { read: _.Jf }));
				this.PUa = _.Ni("backButton", Object.assign({}, {}, { read: _.Jf }));
				this.q0a = _.Ni("gotItButton", Object.assign({}, {}, { read: _.Jf }));
			}
			ib() {
				var a = this.step();
				var b = this.Vp();
				var c = this.Yca();
				var d = `slide-${b + 1}`;
				this.Apa = `slide-container-${d}`;
				this.yya = `dialog-heading-${d}`;
				this.sYa = `dialog-description-${d}`;
				this.Yab = b > 0;
				this.EKa = b < c - 1;
				if (a.link) {
					this.bEa = a.link.label || this.bEa, this.linkUrl = a.link.url.toString();
				}
				this.stepId = a.stepId;
				b = document.createElement("p");
				_.ud(b, a.html);
				this.FA.push(this.yya);
				this.FA.push(this.sYa);
				b.remove();
			}
			Rb() {
				_.ud(this.Ey().nativeElement, this.step().html);
				var a = [["slideContainer", this.el]];
				var b = this.L5a();
				if (b) {
					a.push(["nextButton", b]);
				}
				if (b = this.q0a()) {
					a.push(["gotItButton", b]);
				}
				if (b = this.rTa()) {
					a.push(["actionButton", b]);
				}
				if (b = this.PUa()) {
					a.push(["backButton", b]);
				}
				a.push(["closeButton", this.pWa()]);
				for (b = 0; b < a.length; b++) {
					let [c, d] = a[b];
					let e = b ? this.A.get(a[b - 1][0]) : undefined;
					this.A.set(c, new Gnd(d, e));
				}
				for (let c of this.A.values()) c.next && c.previous && c.Ga.nativeElement.setAttribute("tabindex", "-1");
				this.el.nativeElement.focus();
			}
			oHa() {
				if (this.Yab) {
					this.navigate(this.Vp() - 1), nnd(this, 1);
				}
			}
			aN() {
				if (this.EKa) {
					this.navigate(this.Vp() + 1), nnd(this, 2);
				}
			}
			navigate(a) {
				this.navigation.emit(a);
			}
		};
		Hnd.J = function(a) {
			return new (a || Hnd)();
		};
		Hnd.ka = _.u({
			type: Hnd,
			da: [["xap-tour-step-overlay"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.Ey, And, 5, _.Jf)(b.pWa, Bnd, 5, _.Jf)(b.rTa, Cnd, 5, _.Jf)(b.L5a, Dnd, 5, _.Jf)(b.PUa, End, 5, _.Jf)(b.q0a, Fnd, 5, _.Jf);
				}
				if (a & 2) {
					_.ki(6);
				}
			},
			eb: [
				"role",
				"group",
				"aria-roledescription",
				"carousel",
				"tabIndex",
				"-1",
				1,
				"xap-tour-slide"
			],
			Ua: 9,
			Ja: function(a, b) {
				if (a & 1) {
					_.J("keydown.tab", function(c) {
						return J3(b, c, "slideContainer");
					})("keydown.esc", function() {
						return rnd(b);
					});
				}
				if (a & 2) {
					_.Ch("id", b.Apa), _.wh("aria-label", b.Vp() + 1 + " of " + b.Yca())("aria-describedby", b.FA), _.P("xap-tour-theme-reach-white", b.HC === b.xQa.ipb)("xap-tour-theme-reach-blue", b.HC === b.xQa.Y1b)("xap-tour-slide-animations-disabled", b.DK);
				}
			},
			inputs: {
				step: [1, "step"],
				Vp: [1, "stepIndex"],
				Yca: [1, "tourLength"],
				onNavigation: [1, "onNavigation"],
				showGotItButton: [1, "showGotItButton"]
			},
			outputs: {
				navigation: "navigation",
				exit: "exit"
			},
			ha: 18,
			ia: 7,
			la: () => [
				["contentContainer", ""],
				["closeButton", ""],
				["actionButton", ""],
				["backButton", ""],
				["nextButton", ""],
				["gotItButton", ""],
				" Back",
				" Next ",
				" Got it ",
				[1, "xap-tour-slide-top-container"],
				[1, "xap-tour-slide-content-container"],
				[
					1,
					"xap-tour-slide-header",
					3,
					"id"
				],
				[
					1,
					"xap-tour-slide-content",
					3,
					"id"
				],
				[
					"aria-label",
					"Close guided tour",
					"mat-icon-button",
					"",
					1,
					"xap-tour-exit-button",
					3,
					"click",
					"keydown"
				],
				[1, "overlay-icon"],
				[1, "xap-tour-slide-footer"],
				[1, "xap-tour-step-list-container"],
				[
					"role",
					"presentation",
					"aria-hidden",
					"true",
					1,
					"xap-tour-step-list"
				],
				[1, "xap-tour-slide-controls"],
				[
					"aria-label",
					"Further resources",
					"mat-button",
					"",
					"buttonFormat",
					"",
					"target",
					"_blank",
					1,
					"xap-tour-navigation",
					3,
					"href",
					"buttonType",
					"tourTheme"
				],
				[
					"aria-label",
					"Previous slide",
					"mat-button",
					"",
					"buttonFormat",
					"",
					1,
					"xap-tour-navigation",
					"xap-tour-back-button",
					3,
					"buttonType",
					"tourTheme"
				],
				[
					"mat-button",
					"",
					"buttonFormat",
					"",
					"aria-label",
					"Next slide",
					1,
					"xap-tour-navigation",
					"xap-tour-next-button",
					3,
					"buttonType",
					"tourTheme"
				],
				[
					"mat-button",
					"",
					"buttonFormat",
					"",
					"aria-label",
					"Close guided tour",
					1,
					"xap-tour-navigation",
					"xap-tour-got-it-button",
					3,
					"buttonType",
					"tourTheme"
				],
				[1, "xap-tour-slide-title"],
				[
					1,
					"xap-tour-step-list-bubble",
					3,
					"current"
				],
				[1, "xap-tour-step-list-bubble"],
				[
					"aria-label",
					"Further resources",
					"mat-button",
					"",
					"buttonFormat",
					"",
					"target",
					"_blank",
					1,
					"xap-tour-navigation",
					3,
					"click",
					"keydown",
					"href",
					"buttonType",
					"tourTheme"
				],
				[
					"aria-label",
					"Previous slide",
					"mat-button",
					"",
					"buttonFormat",
					"",
					1,
					"xap-tour-navigation",
					"xap-tour-back-button",
					3,
					"click",
					"keydown",
					"buttonType",
					"tourTheme"
				],
				[
					"mat-button",
					"",
					"buttonFormat",
					"",
					"aria-label",
					"Next slide",
					1,
					"xap-tour-navigation",
					"xap-tour-next-button",
					3,
					"click",
					"keydown",
					"buttonType",
					"tourTheme"
				],
				[
					"mat-button",
					"",
					"buttonFormat",
					"",
					"aria-label",
					"Close guided tour",
					1,
					"xap-tour-navigation",
					"xap-tour-got-it-button",
					3,
					"click",
					"keydown",
					"buttonType",
					"tourTheme"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Gh(0), _.F(1, "div", 9)(2, "div", 10), _.B(3, jnd, 3, 2, "h2", 11), _.I(4, "section", 12, 0), _.H(), _.F(6, "button", 13, 1), _.J("click", function() {
						return rnd(b);
					})("keydown", function(c) {
						return J3(b, c, "closeButton");
					}), _.F(8, "mat-icon", 14), _.R(9, "close"), _.H()()(), _.F(10, "footer", 15)(11, "span", 16), _.B(12, mnd, 3, 1, "ol", 17), _.H(), _.F(13, "span", 18), _.B(14, ond, 3, 4, "a", 19), _.B(15, pnd, 3, 3, "button", 20), _.B(16, qnd, 3, 3, "button", 21), _.B(17, snd, 3, 3, "button", 22), _.H()(), _.Hh();
				}
				if (a & 2) {
					_.y(3), _.C(b.step().title ? 3 : -1), _.y(), _.E("id", b.sYa), _.y(8), _.C(b.Yca() > 1 ? 12 : -1), _.y(2), _.C(b.linkUrl ? 14 : -1), _.y(), _.C(b.Yab ? 15 : -1), _.y(), _.C(b.EKa ? 16 : -1), _.y(), _.C(b.showGotItButton() && !b.EKa ? 17 : -1);
				}
			},
			dependencies: [
				_.tz,
				_.VC,
				_.UC,
				_.XB,
				_.YB,
				_.yA,
				_.xA,
				_.HB,
				znd
			],
			styles: ["@charset \"UTF-8\";.xap-tour-slide{-webkit-animation:xap-tour-step-overlay-appear .13s cubic-bezier(.4,0,.2,1);animation:xap-tour-step-overlay-appear .13s cubic-bezier(.4,0,.2,1);border-radius:8px;display:block;max-width:350px;min-width:200px;padding:10px;position:relative}.xap-tour-slide.xap-tour-slide-animations-disabled{-webkit-animation:none;animation:none}.xap-tour-slide,.xap-tour-slide .overlay-icon{color:#fff}.xap-tour-slide:before{content:\"\";display:block;height:12px;position:absolute;-webkit-transform:rotate(45deg);transform:rotate(45deg);width:12px}.xap-tour-slide[data-overlay-y=center]:before{margin-top:-6px;top:50%}.xap-tour-slide[data-overlay-y=center][data-overlay-x=start]{left:6px}.xap-tour-slide[data-overlay-y=center][data-overlay-x=start]:before{left:-6px}.xap-tour-slide[data-overlay-y=center][data-overlay-x=end]{right:6px}.xap-tour-slide[data-overlay-y=center][data-overlay-x=end]:before{right:-6px}.xap-tour-slide[data-overlay-y=bottom]{bottom:6px}.xap-tour-slide[data-overlay-y=bottom]:before{bottom:-6px}.xap-tour-slide[data-overlay-y=top]{top:6px}.xap-tour-slide[data-overlay-y=top]:before{top:-6px}.xap-tour-slide[data-overlay-y=bottom][data-overlay-x=start],.xap-tour-slide[data-overlay-y=top][data-overlay-x=start]{left:-30px}.xap-tour-slide[data-overlay-y=bottom][data-overlay-x=start]:before,.xap-tour-slide[data-overlay-y=top][data-overlay-x=start]:before{left:24px}.xap-tour-slide[data-overlay-y=bottom][data-overlay-x=center]:before,.xap-tour-slide[data-overlay-y=top][data-overlay-x=center]:before{left:50%;margin-left:-6px}.xap-tour-slide[data-overlay-y=bottom][data-overlay-x=end],.xap-tour-slide[data-overlay-y=top][data-overlay-x=end]{right:-30px}.xap-tour-slide[data-overlay-y=bottom][data-overlay-x=end]:before,.xap-tour-slide[data-overlay-y=top][data-overlay-x=end]:before{right:24px}.xap-tour-slide-top-container{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;padding-left:10px}.xap-tour-slide-header{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;margin:0}.xap-tour-slide-content-container{padding-top:10px}.xap-tour-slide-title{margin-top:0}.xap-tour-slide-content{margin-bottom:8px}.xap-tour-slide-footer{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;padding-left:10px}.xap-tour-slide-footer .xap-tour-step-list-container{width:70px;overflow:hidden}.xap-tour-slide-footer .xap-tour-step-list{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;list-style:none;margin:0;padding:0;white-space:nowrap}.xap-tour-slide-footer .xap-tour-step-list .xap-tour-step-list-bubble{display:inline;opacity:.5;outline:none}.xap-tour-slide-footer .xap-tour-step-list .xap-tour-step-list-bubble:before{content:\" ● \"}.xap-tour-slide-footer .xap-tour-step-list .xap-tour-step-list-bubble.current{opacity:1}.xap-tour-exit-button{margin:-4px -4px 0}.xap-tour-slide-controls{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-pack:end;-webkit-justify-content:end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:end;padding-right:8px;white-space:nowrap}.xap-tour-slide-controls .xap-tour-navigation{max-width:120px}@-webkit-keyframes xap-tour-step-overlay-appear{0%{opacity:0;-webkit-transform:translateY(-4px);transform:translateY(-4px)}to{opacity:1;-webkit-transform:translateY(0);transform:translateY(0)}}@keyframes xap-tour-step-overlay-appear{0%{opacity:0;-webkit-transform:translateY(-4px);transform:translateY(-4px)}to{opacity:1;-webkit-transform:translateY(0);transform:translateY(0)}}"],
			Ab: 2
		});
		var Lnd;
		var Jnd;
		var Knd;
		Ind = [
			{
				Bb: "start",
				Gb: "center",
				Lb: "end",
				Mb: "center"
			},
			{
				Bb: "end",
				Gb: "center",
				Lb: "start",
				Mb: "center"
			},
			{
				Bb: "center",
				Gb: "top",
				Lb: "center",
				Mb: "bottom"
			},
			{
				Bb: "end",
				Gb: "top",
				Lb: "center",
				Mb: "bottom"
			},
			{
				Bb: "start",
				Gb: "top",
				Lb: "center",
				Mb: "bottom"
			},
			{
				Bb: "center",
				Gb: "bottom",
				Lb: "center",
				Mb: "top"
			},
			{
				Bb: "end",
				Gb: "bottom",
				Lb: "center",
				Mb: "top"
			},
			{
				Bb: "start",
				Gb: "bottom",
				Lb: "center",
				Mb: "top"
			}
		];
		Lnd = function(a, b, c) {
			Jnd(a);
			var d = _.Ff(a.destroyed, a.Vp.pipe(_.Gf((f) => f !== c)), a.F.pipe(_.Gf((f) => f !== b))).pipe(_.Qg());
			var e = b.steps[c];
			a.R.observe(e.elementId).pipe(gnd((f) => _.Ff(f.pipe(_.Qg(), _.Zha(e.timeout || 1e3)), f.pipe(_.$g()))), _.dh(d)).subscribe({
				next: (f) => {
					if (f) {
						var g;
						if (!((g = a.H) == null)) {
							g.classList.remove("xap-tour-element-active");
						}
						a.H = f;
						var k;
						if (!((k = a.H) == null)) {
							k.classList.add("xap-tour-element-active");
						}
						f.scrollIntoView({
							behavior: "smooth",
							block: "nearest"
						});
						var p = b.steps;
						var r = b.onNavigation;
						var v = p[c];
						Knd(a, f, v, c, p.length, r, b.showGotItButton, b.disableBackgroundInteractions);
						_.Af(f, "click").pipe(_.dh(d)).subscribe(() => {
							if (!(r == null)) {
								r(c < p.length - 1 ? 4 : 5, v == null ? undefined : v.stepId);
							}
							Jnd(a);
							setTimeout(() => {
								a.Vp.next(c + 1);
							}, 1e3);
						});
						var w = new IntersectionObserver((D) => {
							var { height: G, right: L } = D[0].boundingClientRect;
							if (!G || L <= 0) {
								let N;
								if (!((N = a.H) == null)) {
									N.classList.remove("xap-tour-element-active");
								}
								a.H = undefined;
								w.disconnect();
								a.Vp.next(c + 1);
							}
						});
						w.observe(f);
						d.subscribe(() => {
							w.disconnect();
						});
					} else a.Vp.next(c + 1);
				},
				error: () => {
					a.Vp.next(c + 1);
				}
			});
		};
		_.M3 = function(a) {
			var b;
			if (!((b = a.H) == null)) {
				b.classList.remove("xap-tour-element-active");
			}
			Jnd(a);
			a.F.next(undefined);
			var c;
			if (!((c = a.U) == null)) {
				c.focus();
			}
		};
		_.N3 = function(a, b) {
			return a.I.contains(b);
		};
		Jnd = function(a) {
			var b;
			if ((b = a.Nb) == null ? 0 : b.Ug()) {
				a.Nb.detach();
				a.A = undefined;
			}
		};
		Knd = function(a, b, c, d, e, f, g, k) {
			Mnd(a, k);
			b = Nnd(a, b);
			_.pB(a.Nb, b);
			b.cb.pipe(_.dh(a.destroyed)).subscribe((p) => {
				if (a.A) {
					Object.assign(a.A.location.nativeElement.dataset, {
						overlayX: p.A.Bb,
						overlayY: p.A.Gb
					});
				}
			});
			a.A.zk("step", c);
			a.A.zk("stepIndex", d);
			a.A.zk("tourLength", e);
			a.A.zk("onNavigation", f);
			a.A.zk("showGotItButton", g);
			_.Bu(a.A.ti);
			a.Nb.Cj();
		};
		Mnd = function(a, b) {
			if (!a.Nb) {
				a.Nb = a.overlay.create({
					HD: true,
					uf: b != null ? b : false,
					Rc: "xap-tour-step-overlay",
					zj: a.overlay.A.A({ autoClose: false })
				}), a.Nb.Nn.classList.add("xap-tour"), a.Nb.Nn.setAttribute("role", "dialog"), a.Nb.Nn.setAttribute("aria-label", "Guided Tour"), a.Nb.Nn.setAttribute("aria-modal", "false");
			}
			if (!a.A) {
				a.Nb.detach();
				a.A = a.Nb.attach(new _.xB(Hnd));
				let c = hnd(a.A.instance.navigation).pipe(_.dh(a.destroyed)).subscribe((d) => {
					a.Vp.next(d);
				});
				a.A.instance.exit.subscribe(() => {
					c.unsubscribe();
					_.M3(a);
				});
			}
		};
		Nnd = function(a, b) {
			return _.qB(_.sB(_.rB(_.vB(a.overlay.position(), b), true), true), Ind);
		};
		_.O3 = class {
			constructor() {
				this.R = _.m(L3);
				this.overlay = _.m(_.EB);
				this.I = _.m(_.eGb);
				this.destroyed = new _.Zg();
				this.F = new _.Zg(1);
				this.Vp = new _.Zg(1);
				this.Fi = new _.Zg(1);
				_.vf([this.F.pipe(_.Gf(Boolean)), this.Vp]).pipe(_.dh(this.destroyed)).subscribe(([a, b]) => {
					if (a.steps[b]) {
						Lnd(this, a, b);
					} else {
						_.M3(this);
					}
				});
				_.vf([this.Vp, this.F]).pipe(_.Gla(([a, b]) => {
					var c;
					var d;
					return (a = b == null ? undefined : (c = b.steps[a]) == null ? undefined : (d = c.transition) == null ? undefined : d.duration) ? _.Df(a) : _.ESa;
				}), _.dh(this.destroyed)).subscribe(([a]) => {
					this.Vp.next(a + 1);
				});
				fnd(this.Fi, this.F.pipe(_.Gf((a) => !a), _.bh(undefined))).pipe(_.uf((a) => a[0]), _.dh(this.destroyed)).subscribe((a) => {
					if (_.N3(this, a)) {
						this.F.next(undefined);
					} else {
						this.I.add(a), this.Vp.next(0), this.F.next(a);
					}
				});
			}
			Ba() {
				Jnd(this);
				this.F.complete();
				this.Vp.complete();
				this.Fi.complete();
				this.destroyed.next();
				this.destroyed.complete();
			}
		};
		_.O3.J = function(a) {
			return new (a || _.O3)();
		};
		_.O3.sa = _.Cd({
			token: _.O3,
			factory: _.O3.J,
			wa: "root"
		});
		_.P3 = class {
			constructor() {
				this.el = _.m(_.Jf);
				this.registry = _.m(L3);
				this.xapTourElementId = _.Mi();
				this.teb = _.V(false);
			}
			fT() {
				var a = this.xapTourElementId();
				if (a && this.A !== a) {
					this.unregister(), this.registry.add(a, this.el.nativeElement, this.teb()), this.A = a;
				}
			}
			unregister() {
				if (this.A) {
					this.registry.remove(this.A);
				}
			}
			Ba() {
				this.unregister();
			}
		};
		_.P3.J = function(a) {
			return new (a || _.P3)();
		};
		_.P3.Oa = _.We({
			type: _.P3,
			da: [[
				"",
				"xapTourElementId",
				""
			]],
			inputs: {
				xapTourElementId: [1, "xapTourElementId"],
				teb: [1, "xapTourKeepFirstElement"]
			},
			outputs: { xapTourElementId: "xapTourElementIdChange" }
		});
		var Pnd;
		var ood;
		var pod;
		var rod;
		_.Ond = function() {
			return `Video ${_.Qk(Date.now(), "MMMM dd, yyyy - h:mma", "en-US")}.mp4`;
		};
		Qnd = function(a, b) {
			return _.x(function* () {
				var c = yield Promise.all(b.map((f) => _.rH(a, f, false)));
				c = (yield Promise.all(c.map((f) => f == null ? undefined : f.json()))).filter((f) => !!f).filter((f) => f.name.startsWith("video_generation_"));
				var d = yield Promise.all(c.map((f) => Pnd(a, f.id)));
				var e = yield Promise.all(d.map((f) => f == null ? undefined : f.json()));
				return new Map(c.map((f, g) => {
					var k;
					return [f.id, (k = e[g]) == null ? undefined : k.id];
				}));
			});
		};
		_.Rnd = function(a) {
			if (!a) return "";
			a = Math.round((Date.now() - a.getTime()) / 1e3);
			return a < 60 ? "Just now" : a < 120 ? "A minute ago" : a < 3600 ? `${Math.floor(a / 60)} minutes ago` : Math.floor(a / 3600) === 1 ? "1 hour ago" : a < 86400 ? `${Math.floor(a / 3600)} hours ago` : a < 172800 ? "Yesterday" : a < 604800 ? `${Math.floor(a / 86400)} days ago` : Math.floor(a / 604800) === 1 ? "1 week ago" : a < 2592e3 ? `${Math.floor(a / 604800)} weeks ago` : Math.floor(a / 2592e3) === 1 ? "1 month ago" : a < 31536e3 ? `${Math.floor(a / 2592e3)} months ago` : Math.floor(a / 31536e3) === 1 ? "1 year ago" : `${Math.floor(a / 31536e3)} years ago`;
		};
		Snd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "mat-checkbox", 7);
				_.J("change", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.nCa.set(c.checked));
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("checked", a.nCa()), _.y(), _.U(a.Hjb);
			}
		};
		Tnd = function(a) {
			if (a & 1) {
				_.I(0, "mat-spinner", 8), _.R(1, " Deleting ");
			}
		};
		Und = function(a) {
			if (a & 1) {
				_.R(0, " Delete ");
			}
		};
		Vnd = function(a) {
			if (a & 1) {
				_.F(0, "div", 2), _.I(1, "mat-spinner", 4), _.F(2, "span"), _.R(3, "Fetching prompt details. Please wait..."), _.H()();
			}
			if (a & 2) {
				_.y(), _.E("diameter", 16);
			}
		};
		Wnd = function(a) {
			if (a & 1) {
				_.F(0, "p", 8), _.R(1, "Cannot be empty or contain only spaces."), _.H();
			}
		};
		Xnd = function(a) {
			if (a & 1) {
				_.F(0, "p", 8), _.R(1, "Cannot be more than 100 characters."), _.H();
			}
		};
		Ynd = function(a) {
			if (a & 1) {
				_.F(0, "p", 8), _.R(1, "Cannot be more than 1000 characters."), _.H();
			}
		};
		Znd = function(a) {
			if (a & 1) {
				_.R(0, " Prompt is being processed for saving. Please wait... ");
			}
		};
		$nd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "form")(1, "div", 5)(2, "label", 6);
				_.R(3, "Prompt name");
				_.H();
				_.F(4, "input", 7);
				_.J("input", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.REb(c));
				})("keydown.enter", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.xm());
				});
				_.H();
				_.B(5, Wnd, 2, 0, "p", 8)(6, Xnd, 2, 0, "p", 8);
				_.H();
				_.F(7, "div", 9)(8, "label", 10);
				_.R(9, "Description");
				_.H();
				_.F(10, "textarea", 11);
				_.J("input", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.QEb(c));
				});
				_.R(11, "          ");
				_.H();
				_.B(12, Ynd, 2, 0, "p", 8);
				_.H()();
				_.B(13, Znd, 1, 0);
			}
			if (a & 2) {
				a = _.K();
				let b = _.Vh(0);
				_.y(4);
				_.E("value", a.Baa());
				_.y();
				_.C(a.A3a() ? 5 : a.B3a() ? 6 : -1);
				_.y(5);
				_.E("value", a.zaa());
				_.y(2);
				_.C(a.K2a() ? 12 : -1);
				_.y();
				_.C(b ? 13 : -1);
			}
		};
		aod = function(a) {
			if (a & 1) {
				_.I(0, "mat-spinner", 14), _.R(1, " Processing ");
			}
		};
		bod = function(a) {
			if (a & 1) {
				_.I(0, "mat-spinner", 14), _.R(1, " Saving ");
			}
		};
		cod = function(a) {
			if (a & 1) {
				_.R(0, " Save ");
			}
		};
		dod = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "mat-dialog-actions", 3)(1, "button", 12);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.Ue());
				});
				_.R(2, "Cancel");
				_.H();
				_.F(3, "button", 13);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.xm());
				});
				_.B(4, aod, 2, 0)(5, bod, 2, 0)(6, cod, 1, 0);
				_.H()();
			}
			if (a & 2) {
				a = _.K();
				let b = _.Vh(0);
				_.y(3);
				_.E("disabled", a.AIb());
				_.y();
				_.C(b ? 4 : a.kk() ? 5 : 6);
			}
		};
		fod = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 2);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(eod(c));
				});
				_.I(1, "span", 3);
				_.R(2, " Share ");
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("disabled", a.yc())("matTooltip", a.rd()), _.y(), _.E("iconName", a.S.hQa);
			}
		};
		god = function(a) {
			if (a & 1) {
				_.R(0, " Share ");
			}
		};
		hod = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 4);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(!c.yc() && eod(c));
				});
				_.B(1, god, 1, 0);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("variant", a.nra() ? "borderless" : "icon-borderless")("disabled", a.yc())("matTooltip", a.nra() ? "" : a.rd())("iconName", a.S.hQa), _.y(), _.C(a.nra() ? 1 : -1);
			}
		};
		_.BM.prototype.iu = _.ca(194, function(a, b = false) {
			if (a !== undefined) {
				this.H.set(a);
			} else {
				this.H.update((c) => !c);
			}
			if (!(this.F.A.ey() || b)) {
				this.U.set(this.H());
			}
		});
		iod = function(a) {
			return _.x(function* () {
				if (!(yield _.pf(_.jC(a.dialog.open(_.kF, { data: {
					title: "Save your conversations in Google Drive (Recommended)",
					content: "Saving in Drive makes it easy to keep your work safe in one place, and easily find past conversations.",
					Pba: false,
					dba: "Enable Google Drive",
					T4: "Cancel and use Temporary chat",
					uV: 273916,
					vV: 273917
				} }))))) return a.Ia.hasShownDrivePermissionDialog.set(true), a.Ia.autosaveEnabled.set(false), false;
				var b = yield _.pF(a);
				a.Ia.hasShownDrivePermissionDialog.set(true);
				return b === false ? (a.Ia.autosaveEnabled.set(false), false) : true;
			});
		};
		_.jod = function(a) {
			return _.x(function* () {
				return (yield _.pf(a.H)) ? true : a.Ia.hasShownDrivePermissionDialog() ? false : yield iod(a);
			});
		};
		Pnd = function(a, b) {
			var c = _.Ond();
			return _.x(function* () {
				var d = _.Yx();
				d = yield _.Ey(a.I, d).then((e) => _.l(e, 1)).catch(() => {});
				return fetch(`https://www.googleapis.com/drive/v3/files/${b}/copy`, {
					method: "POST",
					headers: d ? { Authorization: "Bearer " + d } : undefined,
					body: JSON.stringify({ name: c })
				}).then((e) => e).catch(() => null);
			});
		};
		kod = function(a) {
			return a.PC().map((b) => b.Bg).flat().map((b) => b.driveId).filter((b) => !!b);
		};
		lod = function(a) {
			return _.x(function* () {
				var b = a.I().map((d) => d.Bg).flat().map((d) => d.driveId).filter((d) => !!d);
				var c = yield Qnd(a.ma, b);
				b = a.I().map((d) => {
					var e = d.Bg.map((f) => {
						if (f.driveId !== undefined) {
							let g = c.get(f.driveId);
							if (g !== undefined) return Object.assign({}, f, { driveId: g });
						}
						return f;
					});
					return Object.assign({}, d, { Bg: e });
				});
				a.I.set(b);
			});
		};
		_.mod = function(a, b) {
			return _.x(function* () {
				var c = b.getName();
				var d;
				var e = (d = b.getMetadata()) == null ? undefined : d.En();
				yield a.cb.navigate([c], {
					Vq: "merge",
					queryParams: {
						model: null,
						prompt: null,
						resourceKey: e || null,
						save: "true"
					}
				});
			});
		};
		nod = function(a, b, c, d) {
			return _.x(function* () {
				try {
					a.A.set(true);
					_.hn(_.jn(_.Rs(b, _.kn, 5), c), d);
					let e = yield a.H.save(b);
					a.A.set(false);
					return e;
				} catch (e) {
					throw a.A.set(false), e;
				}
			});
		};
		ood = function(a, b, c = []) {
			return _.x(function* () {
				a.R.set(true);
				yield a.H.delete(b, c);
				a.R.set(false);
			});
		};
		pod = function(a) {
			var b = [];
			for (let c of a) {
				let d;
				let e;
				let f;
				let g;
				let k;
				let p;
				let r;
				let v;
				b.push((k = (d = _.fj(c, _.gj, 2, _.sq)) == null ? undefined : d.getId()) != null ? k : "", (p = (e = _.fj(c, _.gj, 3, _.sq)) == null ? undefined : e.getId()) != null ? p : "", (r = (f = _.fj(c, _.gj, 4, _.sq)) == null ? undefined : f.getId()) != null ? r : "", (v = (g = _.fj(c, _.gj, 6, _.sq)) == null ? undefined : g.getId()) != null ? v : "");
			}
			return b.filter((c) => c !== "");
		};
		qod = function(a, b = true) {
			var c = [];
			if (_.Dr(a, _.zq, 14, _.yq)) {
				var d = _.oq(a);
				if (d) {
					d = [..._.pq(d), ..._.mj(d, _.qq, 2, _.oj())], c.push(...pod(d));
				}
			} else if (_.Dr(a, _.Mx, 17, _.yq) && (a = _.fj(a, _.Mx, 17, _.yq))) {
				let e = _.fj(a, _.gj, 2, _.Nx);
				if (e && !b) {
					c.push(e.getId());
				}
				for (d of _.mj(a, _.Ix, 5, _.oj())) (b = _.fj(d, _.gj, 2, _.Jx)) && c.push(b.getId());
			}
			return c;
		};
		rod = function(a, b) {
			return _.x(function* () {
				var c = (yield _.kob(a.R, b.split("/")[1], "Allow access to Google Drive to delete this Prompt.")).getPrompt();
				return qod(c);
			});
		};
		_.Q3 = class {
			constructor() {
				this.F = _.m(_.iC);
				this.Jf = _.m(_.UH);
				this.R = _.m(_.rF);
				this.I = _.m(_.jH);
				this.H = _.m(_.Ou);
				this.rb = _.m(_.Qp);
				this.Rd = _.m(_.uH);
				this.A = _.m(_.qC);
				this.Wa = _.m(_.kC);
				this.Mt = _.M(false);
				this.QB = this.A.af === 17;
				this.Hjb = "Delete generated video/s";
				this.nCa = _.M(true);
			}
			MFa() {
				var a = this;
				return _.x(function* () {
					try {
						let b = a.A.ii;
						let c = a.A.af;
						a.Mt.set(true);
						a.F.show({
							content: "Deleting prompt...",
							Ne: "info",
							Aj: 8
						});
						_.Rn(a.H, "NAV", "Clicked Delete Prompt Button");
						let d = [];
						if (a.nCa() && c === 17) {
							let e = a.rb.name();
							if (b === e) {
								let f = a.Rd.Bg();
								for (let g of f) g.driveId && d.push(g.driveId);
							} else {
								let f = yield rod(a, b);
								d.push(...f);
							}
						}
						yield ood(a.Jf, a.A.ii, d);
						a.F.pj();
						a.I.A.set(true);
						a.Wa.close(true);
					} catch (b) {
						a.Mt.set(false);
						a.F.show({
							content: "Failed to delete prompt. Please retry.",
							Ne: "error",
							Aj: 8
						});
					}
				});
			}
		};
		_.Q3.J = function(a) {
			return new (a || _.Q3)();
		};
		_.Q3.ka = _.u({
			type: _.Q3,
			da: [["ms-prompt-delete-confirmation-dialog"]],
			ha: 13,
			ia: 4,
			la: [
				[1, "action-confirmation"],
				[
					"mat-dialog-title",
					"",
					1,
					"shared-dialog-header"
				],
				[1, "content"],
				[
					1,
					"delete-generated-videos-checkbox",
					3,
					"checked"
				],
				["align", "end"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"mat-dialog-close",
					"",
					3,
					"disabled"
				],
				[
					"ms-button",
					"",
					"cdkFocusRegionEnd",
					"",
					"cdkFocusInitial",
					"",
					3,
					"click",
					"disabled"
				],
				[
					1,
					"delete-generated-videos-checkbox",
					3,
					"change",
					"checked"
				],
				[
					"diameter",
					"16",
					1,
					"delete-progress-indicator"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 0)(1, "h2", 1), _.R(2, " Delete prompt "), _.H(), _.F(3, "mat-dialog-content")(4, "div", 2), _.R(5, "Are you sure?"), _.H(), _.B(6, Snd, 2, 2, "mat-checkbox", 3), _.H(), _.F(7, "mat-dialog-actions", 4)(8, "button", 5), _.R(9, "Cancel"), _.H(), _.F(10, "button", 6), _.J("click", function() {
						return b.MFa();
					}), _.B(11, Tnd, 2, 0)(12, Und, 1, 0), _.H()()();
				}
				if (a & 2) {
					_.y(6), _.C(b.QB ? 6 : -1), _.y(2), _.E("disabled", b.Mt()), _.y(2), _.E("disabled", b.Mt()), _.y(), _.C(b.Mt() ? 11 : 12);
				}
			},
			dependencies: [
				_.Yy,
				_.xC,
				_.sC,
				_.uC,
				_.wC,
				_.vC,
				_.zC,
				_.yC,
				_.qE,
				_.pE
			],
			styles: [".content[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}.delete-progress-indicator[_ngcontent-%COMP%]{margin-right:4px;margin-top:2px}.delete-generated-videos-checkbox[_ngcontent-%COMP%]{margin-top:2px;margin-left:-4px}"]
		});
		_.uod = function(a, b, c) {
			return _.x(function* () {
				try {
					a.Gx.set(true);
					yield sod(a);
					yield tod(a, b, c);
					a.Gx.set(false);
				} catch (d) {
					throw a.Gx.set(false), d;
				}
			});
		};
		tod = function(a, b, c) {
			return _.x(function* () {
				b = b ? _.bGa(b) : a.rb.title();
				c = c !== undefined ? c : a.rb.description();
				switch (a.rb.af()) {
					case 14:
						_.SJ(a.F, {
							title: b,
							description: c
						});
						_.GH(a.rb, b);
						_.qwb(a.rb, c);
						break;
					case 16:
					case 17:
						var d = a.rb.title();
						var e = a.rb.description();
						d = _.Q3a(new _.Rx(), a.rb.name()).A(_.hn(_.jn(new _.kn(), d), e));
						switch (a.rb.af()) {
							case 16:
								e = d;
								var f = a.Hg.F();
								e = _.Ap(e, 16, _.yq, f);
								_.Qx(e, a.H.F());
								break;
							case 17: e = d, f = a.Rd.F(), e = _.Ap(e, 17, _.yq, f), _.Qx(e, a.R.F());
						}
						e = d;
						b && b.length > 0 && _.jn(_.Rs(e, _.kn, 5), b);
						c && c.length > 0 && _.hn(_.Rs(e, _.kn, 5), c);
						d = a.rb.kI();
						e = yield _.Gwb(a.Jf, e);
						e.getName();
						a.rb.name();
						_.swb(a.rb, e);
						d && (a.Gx.set(false), _.Rzb(a.I.service.F, e), yield _.mod(a.rb, e));
				}
			});
		};
		sod = function(a) {
			return _.x(function* () {
				switch (a.rb.af()) {
					case 17: yield lod(a.Rd);
				}
			});
		};
		_.R3 = class {
			constructor() {
				this.rb = _.m(_.Qp);
				_.m(_.Iw);
				_.m(_.jH);
				this.F = _.m(_.fK);
				_.m(_.Zq);
				this.Ia = _.m(_.oF);
				this.Jf = _.m(_.UH);
				this.Hg = _.m(_.hH);
				this.H = _.m(_.dH);
				this.Rd = _.m(_.uH);
				this.R = _.m(_.eH);
				this.NR = _.m(_.iH);
				_.m(_.vH);
				_.m(_.Cl);
				this.A = _.m(_.rF);
				this.dialog = _.m(_.rC);
				_.m(_.Op);
				this.I = _.m(_.NK);
				this.Gx = this.Jf.Gx;
				_.Ck(this.A.Xm, { initialValue: undefined });
				this.U = _.up(() => {
					_.uod(this);
				}, 3e3);
				_.Fk([
					this.Ia.autosaveEnabled,
					this.rb.af,
					this.rb.Aa,
					this.rb.Fa,
					this.rb.SZ,
					this.NR.F
				], () => {
					var a = this;
					return _.x(function* () {
						var b = a.Ia.autosaveEnabled();
						var c = a.rb.af();
						var d = a.rb.Aa();
						var e = a.rb.Fa();
						var f = a.rb.SZ();
						var g = a.NR.F();
						var k = a.Gx();
						if (!(!b || c !== 16 && c !== 17)) {
							b = !k && !d && e && !f, c === 16 && (b = b && !g), b && (c !== 16 || (yield _.jod(a.A))) && a.U();
						}
					});
				});
			}
		};
		_.R3.J = function(a) {
			return new (a || _.R3)();
		};
		_.R3.sa = _.Cd({
			token: _.R3,
			factory: _.R3.J,
			wa: "root"
		});
		var vod;
		vod = function(a, b, c) {
			return _.x(function* () {
				if (a.data.S8) yield _.uod(a.ea, b, c);
				else {
					let d = yield a.lz(a.data.Ll);
					yield nod(a.Jf, d, b, c);
				}
			});
		};
		_.S3 = class {
			constructor() {
				this.Wa = _.m(_.kC);
				this.R = _.m(_.iC);
				this.I = _.m(_.Nw);
				this.data = _.m(_.qC);
				this.H = _.m(_.Ou);
				this.Jf = _.m(_.UH);
				this.rb = _.m(_.Qp);
				this.ea = _.m(_.R3);
				this.aa = _.m(_.jH);
				this.Ii = _.m(_.GJ);
				this.X = _.m(_.gH);
				this.pCa = _.M(false);
				this.A = _.M(false);
				this.Baa = _.M("");
				this.zaa = _.M("");
				this.F = _.W(() => this.Baa().trim());
				this.U = _.W(() => this.zaa().trim());
				this.K2a = _.W(() => this.U().length > 1e3);
				this.A3a = _.W(() => this.F() === "");
				this.B3a = _.W(() => this.F().length > 100);
				this.AIb = _.W(() => this.K2a() || this.B3a() || this.A3a() || this.Ii.Qka() || this.kk());
				this.REb = (a) => {
					this.Baa.set(a.target.value);
				};
				this.QEb = (a) => {
					this.zaa.set(a.target.value);
				};
				this.fa = this.X.kk;
				this.kk = _.W(() => this.Jf.Gx() || this.fa() || this.A());
				if (this.data.S8) {
					this.Baa.set(this.rb.title()), this.zaa.set(this.rb.description());
				} else {
					this.pCa.set(true);
				}
			}
			ib() {
				var a = this;
				return _.x(function* () {
					try {
						if (!a.data.S8) {
							let b = yield a.lz(a.data.Ll);
							let c;
							let d;
							a.Baa.set((d = (c = b.getMetadata()) == null ? undefined : c.getDisplayName()) != null ? d : "");
							let e;
							let f;
							a.zaa.set((f = (e = b.getMetadata()) == null ? undefined : e.jc()) != null ? f : "");
							a.pCa.set(false);
						}
					} catch (b) {
						a.handleError(b);
						a.Wa.close({ save: false });
					}
				});
			}
			xm() {
				var a = this;
				return _.x(function* () {
					if (!a.kk()) try {
						a.A.set(true);
						let b = a.F();
						let c = a.U();
						let d;
						if ((d = a.data) == null ? 0 : d.FG) {
							_.Rn(a.H, a.data.FG, "Clicked Save Dialog Action", "confirm");
						}
						yield vod(a, b, c);
						a.Wa.close({
							save: true,
							title: b,
							description: c
						});
						a.aa.A.set(true);
					} catch (b) {
						a.A.set(false);
						a.handleError(b);
					}
				});
			}
			handleError(a) {
				if (a instanceof Error) {
					this.I.warning(a), this.R.error(a.message);
				} else {
					this.I.warning(Error(String(a))), this.R.error("An unexpected error occurred.");
				}
			}
			lz(a) {
				var b = this;
				return _.x(function* () {
					var c = yield _.TH(b.Jf, a).then((d) => d.prompt);
					if (!c) throw Error("si");
					return c;
				});
			}
			Ue() {
				var a;
				if ((a = this.data) == null ? 0 : a.FG) {
					_.Rn(this.H, this.data.FG, "Clicked Save Dialog Action", "cancel");
				}
				this.Wa.close({ save: false });
			}
		};
		_.S3.J = function(a) {
			return new (a || _.S3)();
		};
		_.S3.ka = _.u({
			type: _.S3,
			da: [["ms-save-prompt-dialog"]],
			ha: 8,
			ia: 3,
			la: [
				[1, "save-prompt-dialog"],
				[
					"mat-dialog-title",
					"",
					1,
					"shared-dialog-header"
				],
				[1, "spinner-container"],
				["align", "end"],
				[3, "diameter"],
				[
					1,
					"title-input-container",
					"form-field"
				],
				[1, "save-dialog-label"],
				[
					"ms-input",
					"",
					"cdkFocusInitial",
					"",
					"aria-label",
					"Prompt name text field",
					3,
					"input",
					"keydown.enter",
					"value"
				],
				[
					"data-test-save-dialog-error-caption",
					"",
					1,
					"error-label"
				],
				[
					1,
					"description-input-container",
					"form-field"
				],
				[
					"for",
					"description-input",
					1,
					"save-dialog-label",
					"description-label"
				],
				[
					"ms-input",
					"",
					"id",
					"description-input",
					"aria-label",
					"Prompt description text field",
					"placeholder",
					"Optional",
					"rows",
					"3",
					1,
					"description-input",
					3,
					"input",
					"value"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"aria-label",
					"Cancel",
					3,
					"click"
				],
				[
					"ms-button",
					"",
					"aria-label",
					"Save title and description",
					3,
					"click",
					"disabled"
				],
				[
					"diameter",
					"16",
					1,
					"save-progress-indicator"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Th(0), _.F(1, "div", 0)(2, "h2", 1), _.R(3, " Save prompt "), _.H(), _.F(4, "mat-dialog-content"), _.B(5, Vnd, 4, 1, "div", 2)(6, $nd, 14, 5), _.H(), _.B(7, dod, 7, 2, "mat-dialog-actions", 3), _.H();
				}
				if (a & 2) {
					_.Uh(b.Ii.Qka()), a = b.pCa(), _.y(5), _.C(a ? 5 : 6), _.y(2), _.C(a ? -1 : 7);
				}
			},
			dependencies: [
				_.Yy,
				_.JD,
				_.wD,
				_.pD,
				_.rD,
				_.gE,
				_.xC,
				_.uC,
				_.wC,
				_.vC,
				_.zC,
				_.yC
			],
			styles: [".base-header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;height:76px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between}.base-header[_ngcontent-%COMP%]   .left-side[_ngcontent-%COMP%], .base-header[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px}.base-header[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{margin-left:12px}@media screen and (max-width:600px){.base-header[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{gap:4px;margin-left:4px}}.dialog-header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;padding:12px 24px}.prompt-header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:2px;height:44px}.prompt-header[_ngcontent-%COMP%]   h3[_ngcontent-%COMP%]{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.prompt-header[_ngcontent-%COMP%]   .left-side[_ngcontent-%COMP%], .prompt-header[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:2px}.prompt-bar[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;bottom:0;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;z-index:3}.prompt-bar[_ngcontent-%COMP%]   .left-side[_ngcontent-%COMP%], .prompt-bar[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px}.center[_ngcontent-%COMP%]{text-align:center}.loading-indicator-container[_ngcontent-%COMP%]{overflow:visible}form[_ngcontent-%COMP%]   label[_ngcontent-%COMP%]{color:var(--color-on-surface);font-weight:500}form[_ngcontent-%COMP%]   label[_ngcontent-%COMP%]   sup[_ngcontent-%COMP%]{line-height:0}form[_ngcontent-%COMP%]   label[_ngcontent-%COMP%]   mat-hint[_ngcontent-%COMP%]{display:block}form[_ngcontent-%COMP%]   mat-checkbox[_ngcontent-%COMP%]{width:100%}form[_ngcontent-%COMP%]   mat-form-field[_ngcontent-%COMP%]{color:var(--color-on-surface);max-width:425px;min-width:425px;width:100%}@media screen and (max-width:768px){form[_ngcontent-%COMP%]   mat-form-field[_ngcontent-%COMP%]{max-width:300px;min-width:300px}}@media screen and (max-width:600px){form[_ngcontent-%COMP%]   mat-form-field[_ngcontent-%COMP%]{max-width:unset;min-width:unset}}form[_ngcontent-%COMP%]   .form-row[_ngcontent-%COMP%]{-webkit-box-align:start;-webkit-align-items:start;-moz-box-align:start;-ms-flex-align:start;align-items:start;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;margin-bottom:48px}@media screen and (max-width:720px){form[_ngcontent-%COMP%]   label[_ngcontent-%COMP%]{-webkit-transform:initial;transform:none}form[_ngcontent-%COMP%]   .form-row[_ngcontent-%COMP%]{-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}}.bold[_ngcontent-%COMP%]{font-weight:700}.link-icon[_ngcontent-%COMP%]{vertical-align:sub}.save-dialog-label[_ngcontent-%COMP%]{color:var(--color-on-surface);margin-bottom:8px}.save-dialog-label.description-label[_ngcontent-%COMP%]{margin-top:8px}.spinner-container[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:8px}.save-progress-indicator[_ngcontent-%COMP%]{margin-right:4px;margin-top:2px}.error-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px}"]
		});
		eod = function(a) {
			return _.x(function* () {
				try {
					if (!a.ii()) throw Error("ti");
					_.Rn(a.H, a.FG(), "Clicked Share Prompt Button");
					if (!(yield _.pf(_.Qvb(a.F)))) throw Error("ui");
					let c = [];
					let d = a.rb.name();
					if (a.ii() === d) {
						if (c.push(...a.Ii.U()), a.rb.af() === 17) {
							let f = kod(a.Rd);
							c.push(...f);
						}
					} else {
						a.A.info("Sharing prompt...");
						c.push(...yield wod(a, a.ii()));
						a.A.pj();
					}
					let e = [a.Ll(), ...c];
					var b = a.F;
					b.F.setItemIds(e);
					b.F.showSettingsDialog();
					if (a.ev && (yield xod())) {
						a.A.success("Link copied to clipboard");
					}
				} catch (c) {
					a.A.error(c.message);
				}
			});
		};
		wod = function(a, b) {
			return _.x(function* () {
				var c = (yield _.kob(a.R, b.split("/")[1], "Allow access to Google Drive to share this Prompt.")).getPrompt();
				return qod(c, false);
			});
		};
		xod = function() {
			return _.x(function* () {
				var a = `${location.origin}${location.pathname}`;
				if (!navigator.clipboard) return console.error("Clipboard API not available."), false;
				try {
					yield navigator.clipboard.writeText(a);
					return true;
				} catch (b) {
					console.error("Failed to copy link to clipboard", b);
					return false;
				}
			});
		};
		_.T3 = class {
			constructor() {
				this.S = _.Dk;
				this.A = _.m(_.iC);
				this.F = _.m(_.sH);
				this.Jf = _.m(_.UH);
				this.R = _.m(_.rF);
				this.H = _.m(_.Ou);
				this.rb = _.m(_.Qp);
				this.Rd = _.m(_.uH);
				this.I = _.m(_.Op);
				this.Ii = _.m(_.GJ);
				this.ii = _.Li.required();
				this.FG = _.Li.required();
				this.C8a = _.V(false);
				this.rd = _.V("");
				this.nra = _.V(false);
				this.yc = _.V(false);
				this.ev = this.I.getFlag(_.vE);
				this.Ll = _.W(() => this.ii().split("/")[1]);
			}
		};
		_.T3.J = function(a) {
			return new (a || _.T3)();
		};
		_.T3.ka = _.u({
			type: _.T3,
			da: [["ms-share-prompt"]],
			inputs: {
				ii: [1, "promptName"],
				FG: [1, "analyticsCategory"],
				C8a: [1, "renderAsMenuItem"],
				rd: [1, "tooltipText"],
				nra: [1, "withLabel"],
				yc: [1, "isDisabled"]
			},
			ha: 2,
			ia: 1,
			la: [
				[
					"mat-menu-item",
					"",
					"disabledInteractive",
					"",
					"matTooltipPosition",
					"after",
					1,
					"icon-text-button",
					3,
					"disabled",
					"matTooltip"
				],
				[
					"ms-button",
					"",
					"aria-label",
					"Share prompt",
					"matTooltipPosition",
					"below",
					3,
					"variant",
					"disabled",
					"matTooltip",
					"iconName"
				],
				[
					"mat-menu-item",
					"",
					"disabledInteractive",
					"",
					"matTooltipPosition",
					"after",
					1,
					"icon-text-button",
					3,
					"click",
					"disabled",
					"matTooltip"
				],
				[3, "iconName"],
				[
					"ms-button",
					"",
					"aria-label",
					"Share prompt",
					"matTooltipPosition",
					"below",
					3,
					"click",
					"variant",
					"disabled",
					"matTooltip",
					"iconName"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, fod, 3, 3, "button", 0)(1, hod, 2, 5, "button", 1);
				}
				if (a & 2) {
					_.C(b.C8a() ? 0 : 1);
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				_.wI,
				_.sI,
				_.IC,
				_.HC
			],
			styles: [".icon-text-button[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{margin-right:8px}"]
		});
		var zod;
		var Aod;
		_.U3 = function(a) {
			var b;
			var c;
			var d;
			return (d = (b = a.getMetadata()) == null ? undefined : (c = b.Aq()) == null ? undefined : _.Pm(c, 2)) != null ? d : false;
		};
		_.yod = function(a) {
			a: {
				if (a.F() && (a = a.getMetadata(), _.sn(a, _.Hx, 5) && (a = _.Z(a, _.Hx, 5), _.sn(a, _.Zo, 1)))) {
					a = _.Z(a, _.Zo, 1).toDate();
					break a;
				}
				a = null;
			}
			return a ? a.getTime() : 0;
		};
		zod = function(a) {
			switch (_.Px(a)) {
				case 2: return "Freeform prompt";
				case 14:
				case 7: return "Chat prompt";
				case 16: return "Imagen prompt";
				case 17: return "Veo prompt";
				default: return "Unknown prompt";
			}
		};
		Aod = function(a) {
			switch (_.Px(a)) {
				case 2: return "format_image_right";
				case 14:
				case 7: return "chat_bubble";
				case 16: return "dashboard";
				default: return "draft";
			}
		};
		_.Bod = function(a) {
			var b = a.getMetadata().Aq();
			if (_.Pm(b, 2)) a = "me";
			else {
				let c;
				let d;
				let e;
				a = (e = (c = a.getMetadata()) == null ? undefined : (d = c.Aq()) == null ? undefined : d.getDisplayName()) != null ? e : "";
			}
			return a;
		};
		_.Cod = class {
			constructor(a) {
				this.prompt = a;
			}
			getPrompt() {
				return this.prompt;
			}
			getId() {
				return this.prompt.getName();
			}
			getName() {
				var a;
				var b;
				return (b = (a = this.prompt.getMetadata()) == null ? undefined : a.getDisplayName()) != null ? b : "";
			}
			jc() {
				var a;
				var b;
				return (b = (a = this.prompt.getMetadata()) == null ? undefined : a.jc()) != null ? b : "";
			}
			Aq() {
				return _.Bod(this.prompt);
			}
			getType() {
				return zod(this.prompt);
			}
			Tg() {
				return Aod(this.prompt);
			}
			vB() {
				return this.prompt.getName();
			}
			iE() {
				return _.yod(this.prompt);
			}
		};
		_.Cod.prototype.P7 = _.ba(82);
		_.Cod.prototype.vR = _.ba(56);
		_.Dod = function(a, b = false, c = true) {
			return _.x(function* () {
				a.Sa.set(true);
				{
					let d = yield a.F.list({
						fVb: c,
						maxResults: b ? 5 : Infinity
					});
					let e = [];
					for (let f of d) e.push(new _.Cod(f));
					a.A.set(e);
				}
				a.Sa.set(false);
			});
		};
		_.V3 = class {
			constructor() {
				this.F = _.m(_.rF);
				this.A = _.M([]);
				_.M("");
				_.M("");
				_.M("");
				this.Sa = _.M(false);
				this.nx = _.M("me");
				this.fca = _.M({
					active: "updated",
					direction: "desc"
				});
				this.AHa = _.W(() => this.A().filter((a) => a.getPrompt() !== null).map((a) => a.getPrompt()));
				this.UM = _.W(() => this.A().filter((a) => _.U3(a.prompt)).sort((a, b) => b.iE() - a.iE()).slice(0, 5).map((a) => {
					var b;
					return {
						id: a.getId(),
						text: a.getName(),
						icon: (b = a.Tg()) != null ? b : undefined,
						routerLink: a.vB(),
						af: _.Px(a.getPrompt()),
						lastModified: a.iE()
					};
				}));
			}
			Wd() {
				var a = this;
				return _.x(function* () {
					return _.pf(a.F.Xm);
				});
			}
		};
		_.V3.J = function(a) {
			return new (a || _.V3)();
		};
		_.V3.sa = _.Cd({
			token: _.V3,
			factory: _.V3.J,
			wa: "root"
		});
		var God;
		Eod = function(a) {
			return _.x(function* () {
				var b = a.A() ? {
					S8: true,
					FG: "LIBRARY"
				} : {
					S8: false,
					Ll: a.Ll(),
					FG: "LIBRARY"
				};
				a.dialog.open(_.S3, { data: b });
			});
		};
		Fod = function(a) {
			return _.x(function* () {
				var b = {
					ii: a.ii(),
					af: a.af()
				};
				if ((yield _.pf(_.jC(a.dialog.open(_.Q3, { data: b })))) && a.A()) {
					_.br(a.I, "library");
				}
			});
		};
		God = function(a, b, c = "info") {
			if (c === "error") {
				a.F.error(b);
			} else {
				a.F.info(b);
			}
		};
		Hod = function(a) {
			return _.x(function* () {
				var b = a.A() ? a.rb.title() : a.Aaa().text;
				God(a, `Copying "${b}"...`);
				if (a.A()) a.H.Hxa({
					title: `Copy of ${a.rb.title()}`,
					description: a.rb.description()
				});
				else try {
					let c = yield _.TH(a.Jf, a.Ll());
					if (c.prompt) {
						let d = _.Q3a(c.prompt.clone(), "");
						let e;
						let f;
						let g = (f = (e = d.getMetadata()) == null ? undefined : e.getDisplayName()) != null ? f : a.Aaa().text;
						_.jn(_.Rs(d, _.kn, 5), `Copy of ${g}`);
						let k = yield _.Gwb(a.Jf, d);
						if (k) {
							yield _.mod(a.rb, k);
						}
					} else God(a, "Failed to load prompt.", "error");
				} catch (c) {
					God(a, `Failed to copy prompt. ${(c == null ? undefined : c.message) || ""}`, "error");
				}
			});
		};
		_.W3 = class {
			constructor() {
				this.S = _.Dk;
				this.I = _.m(_.Cl);
				this.dialog = _.m(_.rC);
				this.Jf = _.m(_.UH);
				this.rb = _.m(_.Qp);
				this.H = _.m(_.fK);
				this.F = _.m(_.iC);
				this.Ara = "NAV";
				this.size = _.V("default");
				this.Aaa = _.Li.required();
				this.SF = _.V("below");
				this.GPa = "More options";
				this.ii = _.W(() => this.Aaa().routerLink);
				this.af = _.W(() => this.Aaa().af);
				this.Ll = _.W(() => this.ii().split("/")[1]);
				this.A = _.W(() => this.ii() === this.rb.name());
			}
		};
		_.W3.J = function(a) {
			return new (a || _.W3)();
		};
		_.W3.ka = _.u({
			type: _.W3,
			da: [["ms-prompt-options-menu"]],
			inputs: {
				size: [1, "size"],
				Aaa: [1, "promptItem"],
				SF: [1, "tooltipPosition"]
			},
			ha: 13,
			ia: 12,
			la: [
				["overflowmenu", ""],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					3,
					"click",
					"size",
					"matTooltip",
					"matTooltipPosition",
					"matMenuTriggerFor",
					"iconName"
				],
				[
					"mat-menu-item",
					"",
					1,
					"rename-menu-item",
					3,
					"click"
				],
				[
					1,
					"start-icon",
					3,
					"iconName"
				],
				[
					3,
					"promptName",
					"renderAsMenuItem",
					"analyticsCategory"
				],
				[
					"mat-menu-item",
					"",
					1,
					"copy-menu-item",
					3,
					"click"
				],
				[
					"mat-menu-item",
					"",
					1,
					"delete-menu-item",
					3,
					"click"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "button", 1), _.J("click", function(c) {
						return c.stopPropagation();
					}), _.H(), _.F(1, "mat-menu", null, 0)(3, "button", 2), _.J("click", function() {
						return Eod(b);
					}), _.I(4, "span", 3), _.R(5, " Rename "), _.H(), _.I(6, "ms-share-prompt", 4), _.F(7, "button", 5), _.J("click", function() {
						return Hod(b);
					}), _.I(8, "span", 3), _.R(9, " Make a copy "), _.H(), _.F(10, "button", 6), _.J("click", function() {
						return Fod(b);
					}), _.I(11, "span", 3), _.R(12, " Delete "), _.H()();
				}
				if (a & 2) {
					a = _.O(2), _.E("size", b.size())("matTooltip", b.GPa)("matTooltipPosition", b.SF())("matMenuTriggerFor", a)("iconName", b.S.my), _.wh("aria-label", b.GPa), _.y(4), _.E("iconName", b.S.pn), _.y(2), _.E("promptName", b.ii())("renderAsMenuItem", true)("analyticsCategory", b.Ara), _.y(2), _.E("iconName", b.S.Ae), _.y(3), _.E("iconName", b.S.ni);
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				_.OD,
				_.wI,
				_.tI,
				_.sI,
				_.vI,
				_.IC,
				_.HC,
				_.T3
			],
			Ab: 2
		});
		_.hr("XVDWwe");
		var Lod = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var Mod = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var Ood = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "div", 14);
				_.J("keydown.escape", function(c) {
					_.q(b);
					var d = _.K(2);
					return _.t(d.r5(c));
				});
				_.I(1, "div", 15);
				_.F(2, "div", 16);
				_.z(3, Mod, 1, 0, "ng-container", 12);
				_.F(4, "div", 17);
				_.R(5);
				_.H();
				_.F(6, "div", 18);
				_.R(7);
				_.H();
				_.F(8, "button", 19);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.PKa());
				});
				_.Mh(9, 4);
				_.H();
				_.I(10, "mat-divider", 20);
				_.F(11, "button", 21);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.signOut());
				});
				_.Mh(12, 5);
				_.H();
				_.I(13, "mat-divider", 20);
				_.F(14, "div", 22)(15, "div", 23)(16, "a", 24);
				_.Mh(17, 6);
				_.H()();
				_.F(18, "span", 25);
				_.R(19, "·");
				_.H();
				_.F(20, "div", 23)(21, "a", 26);
				_.Mh(22, 7);
				_.H()()()()();
			}
			if (a & 2) {
				a = _.K(2);
				let b = _.O(5);
				_.P("hidden", !a.Ep);
				_.y(3);
				_.E("ngTemplateOutlet", b)("ngTemplateOutletContext", _.zi(12, Nod));
				_.y(2);
				_.U(a.name);
				_.y(2);
				_.U(a.email);
				_.y(2);
				_.Qh(a.eta);
				_.Rh(9);
				_.y(3);
				_.Qh(a.dta);
				_.Rh(12);
				_.y(4);
				_.E("href", _.wi(a.sHa), _.rg);
				_.y(5);
				_.E("href", _.wi(a.MLa), _.rg);
			}
		};
		var Qod = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "div", 10)(1, "div", 11, 2);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.nO());
				})("keydown.enter", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.V$(c));
				})("keydown.space", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.V$(c));
				})("keydown.escape", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.r5(c));
				});
				_.z(4, Lod, 1, 0, "ng-container", 12);
				_.H();
				_.z(5, Ood, 23, 13, "ng-template", 13);
				_.H();
			}
			if (a & 2) {
				a = _.O(3);
				let b = _.K();
				let c = _.O(5);
				_.y();
				_.P("selected", b.Ep);
				_.E("matTooltip", b.z7());
				_.wh("aria-label", b.z7());
				_.y(3);
				_.E("ngTemplateOutlet", c)("ngTemplateOutletContext", _.zi(9, Pod));
				_.y();
				_.E("cdkConnectedOverlayOrigin", a)("cdkConnectedOverlayPositions", b.G_)("cdkConnectedOverlayOpen", b.Ep);
			}
		};
		var Rod = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 27);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.signIn());
				});
				_.Mh(1, 8);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.Qh(a.cta), _.Rh(1);
			}
		};
		var Sod = function(a) {
			if (a & 1) {
				_.I(0, "connect-avatar", 29);
			}
			if (a & 2) {
				a = _.K().size;
				let b = _.K();
				_.E("size", a)("src", b.photoUrl)("alt", b.name);
			}
		};
		var Tod = function(a) {
			if (a & 1) {
				_.I(0, "connect-monogram", 30);
			}
			if (a & 2) {
				a = _.K().size;
				let b = _.K();
				_.E("initials", b.name.charAt(0))("size", a);
			}
		};
		var Uod = function(a) {
			if (a & 1) {
				_.z(0, Sod, 1, 3, "connect-avatar", 28)(1, Tod, 1, 2, "ng-template", null, 3, _.Ii);
			}
			if (a & 2) {
				a = _.O(2);
				let b = _.K();
				_.E("ngIf", b.photoUrl)("ngIfElse", a);
			}
		};
		var kpd = function(a) {
			if (a & 1) {
				_.F(0, "span", 14), _.R(1), _.H();
			}
			if (a & 2) {
				let b;
				a = _.K().V;
				let c = _.K(2);
				_.E("iconName", c.S.bfb);
				_.y();
				_.S(" ", (b = _.Z(a, jpd, 12)) == null ? null : b.getText(), " ");
			}
		};
		var lpd = function(a, b) {
			if (a & 1) {
				let c = _.n();
				_.F(0, "button", 8, 2);
				_.Ei(2, "buildVeMetadata");
				_.J("click", function() {
					var d = _.q(c).V;
					var e = _.K(2);
					return _.t(e.GFa(d));
				});
				_.F(3, "div", 9);
				_.I(4, "span", 10);
				_.Ei(5, "toIconWithDefault");
				_.H();
				_.F(6, "div", 11)(7, "div", 12);
				_.R(8);
				_.H();
				_.F(9, "div", 13);
				_.R(10);
				_.H();
				_.B(11, kpd, 2, 2, "span", 14);
				_.H()();
			}
			if (a & 2) {
				let c;
				a = b.V;
				b = _.K(2);
				_.E("ve", b.ve.alb)("veClick", true)("veImpression", true)("veMetadata", _.Fi(2, 9, [{
					Hl: 9,
					value: a.getHeader()
				}]))("veMutable", true);
				_.y(4);
				_.E("iconName", _.Gi(5, 11, a.Tg(), b.S.Fk));
				_.y(4);
				_.U(a.getHeader());
				_.y(2);
				_.U(a.jc());
				_.y();
				_.C(((c = _.Z(a, jpd, 12)) == null ? 0 : c.getText()) ? 11 : -1);
			}
		};
		var mpd = function(a) {
			if (a & 1) {
				_.F(0, "div", 7), _.R(1, "No updates"), _.H();
			}
		};
		var npd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "div", 5, 1);
				_.J("keydown", function(c) {
					_.q(b);
					var d = _.K();
					return _.t(d.rE(c));
				})("mouseenter", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.Nr());
				})("mouseleave", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.QU());
				});
				_.Ah(2, lpd, 12, 14, "button", 6, _.zh, false, mpd, 2, 0, "div", 7);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("@popover", undefined)("ve", a.ve.blb)("veImpression", true), _.y(2), _.Bh(a.N7a());
			}
		};
		;
		;
		;
		var Eqd = function(a) {
			if (a & 1) {
				_.I(0, "li", 20);
			}
		};
		;
		;
		;
		;
		var Rrd = function(a) {
			if (a & 1) {
				_.F(0, "li", 31)(1, "div", 33), _.R(2), _.H()();
			}
			if (a & 2) {
				a = _.K().V, _.y(2), _.U(a.category);
			}
		};
		var Srd = function(a) {
			if (a & 1) {
				_.F(0, "li", 31), _.I(1, "mat-divider"), _.H();
			}
		};
		var Trd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "li", 35);
				_.J("click", function(c) {
					_.q(b);
					_.K(8);
					c.stopPropagation();
					return _.t();
				});
				_.I(1, "div", 36);
				_.F(2, "div", 37)(3, "span", 38);
				_.R(4, "\xA0");
				_.H();
				_.F(5, "p", 39);
				_.R(6, "\xA0");
				_.H()()();
			}
		};
		var Vrd = function(a) {
			if (a & 1) {
				_.Ah(0, Trd, 7, 0, "li", 34, _.yh);
			}
			if (a & 2) {
				a = _.K().V, _.K(6), _.Bh(Urd(a.GJb));
			}
		};
		var Wrd = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		var Yrd = function(a) {
			if (a & 1) {
				_.z(0, Wrd, 1, 0, "ng-container", 41);
			}
			if (a & 2) {
				a = _.K(2).V, _.E("ngTemplateOutlet", a.template)("ngTemplateOutletContext", _.Ai(2, Xrd, a.IF || a));
			}
		};
		var Zrd = function(a) {
			if (a & 1) {
				_.F(0, "div", 42), _.I(1, "span", 47), _.H();
			}
			if (a & 2) {
				a = _.K(3).V, _.y(), _.E("iconName", a.icon);
			}
		};
		var $rd = function(a) {
			if (a & 1) {
				_.F(0, "p", 44), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(3).V, _.y(), _.U(a.description);
			}
		};
		var asd = function(a, b) {
			if (a & 1) {
				_.F(0, "kbd"), _.R(1), _.H();
			}
			if (a & 2) {
				a = b.V, _.y(), _.U(a);
			}
		};
		var bsd = function(a) {
			if (a & 1) {
				_.F(0, "div", 45), _.Ah(1, asd, 2, 1, "kbd", null, _.yh), _.H();
			}
			if (a & 2) {
				a = _.K(3).V, _.y(), _.Bh(a.D9);
			}
		};
		var csd = function(a) {
			if (a & 1) {
				_.I(0, "span", 46);
			}
			if (a & 2) {
				a = _.K(9), _.E("iconName", a.S.gh);
			}
		};
		var dsd = function(a) {
			if (a & 1) {
				_.B(0, Zrd, 2, 1, "div", 42), _.F(1, "div", 37)(2, "span", 43), _.R(3), _.H(), _.B(4, $rd, 2, 1, "p", 44), _.H(), _.B(5, bsd, 3, 0, "div", 45), _.B(6, csd, 1, 1, "span", 46);
			}
			if (a & 2) {
				a = _.K(2).V, _.C(a.icon ? 0 : -1), _.y(3), _.U(a.label), _.y(), _.C(a.description ? 4 : -1), _.y(), _.C(a.D9 ? 5 : -1), _.y(), _.C(a.variant === "menu" ? 6 : -1);
			}
		};
		var lsd = function(a) {
			if (a & 1) {
				let c = _.n();
				_.F(0, "li", 40, 4);
				_.Ei(2, "buildVeProtoMetadata");
				_.J("click", function(d) {
					_.q(c);
					var e = _.K().V;
					var f = _.K(6);
					esd(f, e);
					return _.t(f.preventClose(d));
				});
				_.B(3, Yrd, 1, 4, "ng-container")(4, dsd, 7, 5);
				_.H();
			}
			if (a & 2) {
				var b = _.K();
				a = b.V;
				b = b.jb;
				let c = _.K(6);
				_.P("selected", b === c.selectedIndex());
				_.E("id", "omnibar-item-" + b)("ve", c.ve.dob)("veClick", true)("veImpression", true)("veMetadata", _.Fi(2, 11, Frd(new _.gz(), fsd(gsd(hsd(isd(_.y3(new _.z3(), c.gz() ? 1 : 2), a.id), jsd(a.variant)), a.category), b))))("veMutable", true);
				_.wh("aria-selected", b === c.selectedIndex())("aria-label", a.ariaLabel || ksd(a));
				_.y(3);
				_.C(a.template ? 3 : 4);
			}
		};
		;
		var vsd = function(a) {
			if (a & 1) {
				_.I(0, "ms-omnibar", 12);
			}
			if (a & 2) {
				_.E("isOverlay", true);
			}
		};
		var wsd = function(a) {
			if (a & 1) {
				_.F(0, "div", 6)(1, "div", 13), _.I(2, "span", 14), _.F(3, "span"), _.R(4, "You are currently offline"), _.H()()();
			}
			if (a & 2) {
				a = _.K(), _.E("@fadeInOut", undefined), _.y(2), _.E("iconName", a.S.fib);
			}
		};
		var xsd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "div", 15);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.Qc.iu());
				});
				_.H();
			}
			if (a & 2) {
				_.E("@fadeInOut", undefined);
			}
		};
		var ysd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "div", 15);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.Qc.vJ());
				});
				_.H();
			}
			if (a & 2) {
				_.E("@fadeInOut", undefined);
			}
		};
		var zsd = function(a) {
			if (a & 1) {
				_.I(0, "ms-navbar-v2");
			}
		};
		var Asd = function(a) {
			if (a & 1) {
				_.I(0, "ms-navbar");
			}
		};
		var Bsd = function(a) {
			if (a & 1) {
				_.B(0, zsd, 1, 0, "ms-navbar-v2")(1, Asd, 1, 0, "ms-navbar");
			}
			if (a & 2) {
				a = _.K(), _.C(a.aCb ? 0 : 1);
			}
		};
		var Csd = function(a) {
			if (a & 1) {
				_.F(0, "div", 11)(1, "div", 16), _.I(2, "mat-spinner", 17), _.F(3, "span", 18), _.R(4, "Importing app..."), _.H()()();
			}
		};
		var Esd = function(a) {
			return !Dsd.some((b) => a.startsWith(`/${b}`));
		};
		var Fsd = function(a) {
			a.I.set(true);
			a.A.update((b) => {
				if (b) {
					b = b.clone(), b = _.Mj(b, 6, true);
				}
				return b;
			});
		};
		_.s3.prototype.y6 = _.ca(210, function() {
			this.I.set(true);
			var a = this.ewa();
			if (a !== undefined) {
				this.window.localStorage.setItem("bannerVersionViewed", a.toString());
			}
		});
		_.yG.prototype.Tfa = _.ca(172, function(a, b, c = 0, d = 0) {
			var e = this;
			return _.x(function* () {
				var f = new _.H_a();
				f = _.Uc(f, 1, "users/me");
				f = _.gt(f, 3, a);
				f = _.cq(f, 4, b);
				f = _.cq(f, 5, b);
				f = _.ot(f, 6, 1);
				if (c) {
					var g = _.gt(f, 7, c);
					_.ot(g, 9, d);
				}
				g = e.ea;
				f = yield _.$q(g.A, g.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/AcceptTerms", f, {}, _.J_a);
				Fsd(e.H);
				return f;
			});
		});
		var Gsd = class extends _.h {
			constructor(a) {
				super(a);
			}
			getPrompt() {
				return _.Z(this, _.Rx, 2);
			}
			setPrompt(a) {
				return _.ln(this, _.Rx, 2, a);
			}
		};
		var Hsd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var Isd = [1, 2];
		var jpd = class extends _.h {
			constructor(a) {
				super(a);
			}
			getText() {
				return _.l(this, 1);
			}
			setText(a) {
				return _.Uc(this, 1, a);
			}
		};
		var Jsd = class extends _.h {
			constructor(a) {
				super(a);
			}
			getHeader() {
				return _.l(this, 1);
			}
			setHeader(a) {
				return _.Uc(this, 1, a);
			}
			jc() {
				return _.l(this, 2);
			}
			getTag() {
				return _.l(this, 4);
			}
			Tg() {
				return _.l(this, 11);
			}
		};
		var isd = function(a, b) {
			return _.Lj(a, 2, b);
		};
		var hsd = function(a, b) {
			return _.cn(a, 3, b);
		};
		var gsd = function(a, b) {
			return _.Lj(a, 4, b);
		};
		var fsd = function(a, b) {
			return _.Ym(a, 5, b);
		};
		var Ksd = function(a, b) {
			return _.Lj(a, 6, b);
		};
		var Lsd = function(a, b) {
			return _.Ym(a, 7, b);
		};
		var Frd = function(a, b) {
			return _.ln(a, _.z3, 13, b);
		};
		var Nsd = function(a, b) {
			_.x(function* () {
				a.H.set(b);
				var c = new _.sy();
				c = _.ot(c, 13, b ? 1 : 2);
				return _.nF(a, c, ["autocomplete_preference"]);
			});
		};
		var Osd = function(a) {
			_.x(function* () {
				var b = new _.Y9a();
				var c = [];
				yield _.RCa(() => _.x(function* () {
					var d = false;
					var e;
					try {
						yield _.Fmd(a.ta, () => _.x(function* () {
							try {
								c = _.Z9a(yield _.$bb(a.H, b)).map((f) => _.oc(f));
								d = true;
								return { success: true };
							} catch (f) {
								if (f instanceof _.nn) {
									e = _.wdb.has(f.code) ? f : undefined;
								}
								return { success: false };
							}
						}), 6e4);
					} catch (f) {
						return;
					}
					if (!e && !d) throw Error("oc");
				}), 5);
				a.I.set(c);
				_.Ppb(a, c);
				a.U.set(true);
				a.Jo.resolve();
			});
		};
		var Psd = function(a, b) {
			a.F.set(b);
			a.A.update((c) => {
				if (c) {
					c = c.clone(), c = _.Ym(c, 10, b);
				}
				return c;
			});
		};
		var Qsd = function(a) {
			_.x(function* () {
				try {
					let b = yield _.pf(_.xcb(a.Za, "", {
						qZ: false,
						pageSize: 6
					}));
					if (b && b.length !== 0 && b.length <= 5) {
						let c = b.map((d) => d.getName());
						yield a.Za.CB(c);
					}
				} catch (b) {
					console.warn("Failed to auto-import projects", b);
				}
			});
		};
		var Ssd = class {
			constructor() {
				this.data = _.m(_.qC);
				this.H = _.m(_.uG);
				this.vsa = _.m(_.Ou);
				this.Wa = _.m(_.kC);
				this.F = _.m(_.Op);
				this.A = _.m(_.rF);
				this.Wd = _.Ck(this.A.Xm, { initialValue: undefined });
				this.xqa = _.M(false);
				this.I = this.F.getFlag(_.ytb);
				this.hz = this.H.hz();
				var a;
				this.Zka = ((a = this.data.Zka) != null ? a : false) && (0, _.Jp)();
				var b;
				this.Rj = (b = this.data.Rj) != null ? b : false;
				this.G1 = new _.qD({
					consentCheckbox: new _.uD(this.Zka, {
						Oq: true,
						CO: this.Zka ? [] : [Msd]
					}),
					emailOptIn: new _.uD(false, { Oq: true })
				});
			}
		};
		Ssd.J = function(a) {
			return new (a || Ssd)();
		};
		Ssd.ka = _.u({
			type: Ssd,
			da: [["ms-tos-dialog"]],
			ha: 13,
			ia: 5,
			la: () => [
				"Welcome to AI Studio",
				" Google AI Studio and the Gemini API enable developers to build with the latest Google AI models like Gemini, Veo, Lyria, and more. ",
				"�*12:1��#1:1��/#1:1��/*12:1� Continue ",
				"Agreements",
				"�#4�*�/#4� I acknowledge that I am a developer building with Google AI Studio and Gemini API for professional or business purposes, and I consent to the terms and acknowledge the privacy policy linked above. ",
				" I'd like to receive emails for model updates, offers, useful tips, invitations to participate in research studies, and news about Google AI. ",
				[
					1,
					"tos-dialog",
					3,
					"ngSubmit",
					"formGroup"
				],
				["mat-dialog-content", ""],
				["type", "lockup"],
				[1, "tos-headline"],
				[1, "tos-dialog-content"],
				[3, "showStarterTierTos"],
				[
					"mat-dialog-actions",
					"",
					"align",
					"end"
				],
				[
					"ms-button",
					"",
					"type",
					"submit",
					"aria-label",
					"Accept terms of service",
					3,
					"disabled"
				],
				[3, "diameter"],
				[1, "agreements-label"],
				[
					"cdkFocusInitial",
					"",
					"aria-label",
					"I consent to the Google APIs Terms of Service and the Gemini API Additional Terms\n        of Service and acknowledge that I have read the Google Privacy Policy",
					1,
					"tos-option",
					"align-top",
					3,
					"change",
					"formControl"
				],
				[
					"matTooltip",
					"Required to access the Gemini API and AI Studio",
					"matTooltipPosition",
					"above"
				],
				[
					"aria-label",
					"Opt in to receive news, offers, promotions, and updates about Google AI.",
					1,
					"tos-option",
					"align-top",
					3,
					"formControl"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "form", 6), _.J("ngSubmit", function() {
						return Rsd(b);
					}), _.F(1, "div", 7), _.I(2, "ms-logo-icon", 8), _.F(3, "h1", 9), _.Mh(4, 0), _.H(), _.F(5, "p", 10), _.Mh(6, 1), _.H(), _.I(7, "ms-tos-disclaimer", 11), _.B(8, Iod, 7, 2), _.H(), _.F(9, "div", 12)(10, "button", 13), _.Kh(11, 2), _.B(12, Jod, 2, 1, "mat-spinner", 14), _.Lh(), _.H()()();
				}
				if (a & 2) {
					_.E("formGroup", b.G1), _.y(7), _.E("showStarterTierTos", b.Rj), _.y(), _.C(b.Zka ? -1 : 8), _.y(2), _.E("disabled", b.G1.invalid || b.xqa()), _.y(2), _.C(b.xqa() ? 12 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.tG,
				_.qE,
				_.pE,
				_.wC,
				_.vC,
				_.zC,
				_.yC,
				_.IC,
				_.HC,
				_.MD,
				_.wD,
				_.oD,
				_.pD,
				_.zD,
				_.DD,
				_.wG
			],
			styles: [".tos-dialog-content[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-var)}.tos-dialog-content[_ngcontent-%COMP%]   .no-wrap[_ngcontent-%COMP%]{white-space:nowrap}.agreements-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text)}.tos-option[_ngcontent-%COMP%]{margin-bottom:8px;--mat-checkbox-label-text-size:14px;--mat-checkbox-label-text-font:Inter,sans-serif;--mat-checkbox-label-text-color:var(--color-v3-text-var)}.tos-headline[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:16px;font-weight:600;line-height:24px;color:var(--color-v3-text)}@media screen and (max-width:600px){.tos-headline[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px}}ms-logo-icon[_ngcontent-%COMP%]{display:block;margin:20px auto 24px;width:140px;padding-top:8px}.external-icon[_ngcontent-%COMP%]{margin-left:4px;top:2px}sup[_ngcontent-%COMP%]{font-size:medium;vertical-align:text-top;color:var(--color-v3-accent-3)}"]
		});
		var Tsd = function(a) {
			a = a.H.F();
			return a === undefined || a === 0;
		};
		var Usd = function(a) {
			return _.x(function* () {
				var b = a.F();
				if (b !== undefined) if (a.R || (a.R = true, a.U.set(Tsd(a))), b.Ika) _.Atb(a) && _.Btb(a);
				else {
					_.Rn(a.X, "SIGNUP", "Showed Terms of Service Dialog");
					let c = a.I.getFlag(_.vG);
					_.jC(a.dialog.open(Ssd, {
						id: "tos-dialog",
						data: {
							Tfa: (d) => _.x(function* () {
								yield a.Tfa(b.aU, d, b.eoa, 1);
								yield _.hob(a.Ia);
							}),
							Rj: c
						},
						yi: true,
						width: "481px",
						maxHeight: "90vh"
					})).subscribe(() => {
						var d;
						if ((d = a.F()) == null ? 0 : d.Ika) {
							_.vtb(a.aa);
							if (a.I.getFlag(_.ztb) && Tsd(a)) {
								Qsd(a.ma), Psd(a.H, b.aU);
							}
						}
					});
				}
			});
		};
		var Vsd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var Wsd = function(a, b) {
			var c = a.A[b];
			var d;
			b = (d = a.X.get(c)) != null ? d : [];
			return c === 8 && (a = a.window.localStorage.getItem("bannerFirstSeenDate_" + c.toString())) && Date.now() - Number(a) > 2592e6 ? false : b.every((e) => e());
		};
		var Xsd = function(a) {
			return _.x(function* () {
				var b = new _.b$a();
				var c = a.R;
				b = yield _.$q(c.A, c.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/ListPromos", b, {}, _.d$a);
				var d;
				if (b) {
					d = _.mj(b, Jsd, 1, _.oj());
				} else {
					d = [];
				}
				return d;
			});
		};
		var Zsd = function(a, b) {
			a.commands.update((c) => {
				var d = a.A.get("recent");
				var e = d ? d.items : new Set();
				c = c.filter((g) => !e.has(g.id));
				d = new Set();
				var f = [];
				for (let g of b) d.has(g.id) || (f.push(g), d.add(g.id));
				a.A.set("recent", {
					id: "recent",
					items: d
				});
				return [...c, ...f];
			});
		};
		var dtd = function(a, b) {
			b = ctd(a, b);
			if (!b) return false;
			_.Wmd(a);
			b.action();
			return true;
		};
		var Pod = () => ({ size: "medium" });
		var Nod = () => ({ size: "large" });
		var jtd = function(a) {
			_.Bu(a.ti);
			if (a = document.getElementById("accountSwitcherFirstFocus")) {
				a.focus();
			}
		};
		var ktd = function(a) {
			_.Af(document.body, "click").pipe(_.Qg()).subscribe((b) => {
				b = b.target;
				if (b.closest("#account-switcher") || b.closest("#account-switcher-button")) {
					ktd(a);
				} else {
					a.Ep = false, _.Bu(a.ti);
				}
			});
		};
		var Y3 = class {
			constructor(a, b, c, d, e) {
				this.A = b;
				this.window = c;
				this.F = d;
				this.ti = e;
				this.cta = "Sign in";
				this.dta = "Sign out";
				this.eta = "Switch account";
				this.G_ = _.cFb;
				this.Ep = false;
				this.F3a = new _.ml(true);
				this.Mg = new _.af();
				this.logger = null;
				this.photoUrl = this.email = this.name = "";
				b = Number.isInteger(_.yEb(b)) ? `?authuser=${_.zEb(b)}` : "";
				this.sHa = `https://policies.google.com/privacy${b}`;
				this.MLa = `https://myaccount.google.com/termsofservice${b}`;
				this.Mg.add(_.JEb(a).subscribe((f) => {
					this.name = f[0].names[0];
					this.email = f[0].emailAddresses[0];
					if (f[0].photoUrls[0]) {
						this.photoUrl = new _.Cq(f[0].photoUrls[0], false).lG(64).LV(true).build();
					}
					this.ti.lb();
				}, (f) => {
					if (f.error.status === "UNAUTHENTICATED") {
						this.F3a.next(false);
					}
				}));
			}
			Ba() {
				this.Mg.unsubscribe();
			}
			z7() {
				return "Google Account: " + this.name + " (" + this.email + ")";
			}
			signOut() {
				var a = _.BEb(this.A);
				var b;
				var c = (b = this.F) == null ? undefined : b.jab[a];
				if (!c) {
					c = this.window.location.href;
				}
				a = _.On(new _.hk("https://accounts.google.com/Logout"), "continue", c);
				_.wd(this.window.location, _.kd(a.toString()));
			}
			signIn() {
				var a = _.On(new _.hk("https://accounts.google.com/ServiceLogin"), "continue", this.window.location.href);
				_.wd(this.window.location, _.kd(a.toString()));
			}
			PKa() {
				var a = _.On(_.On(new _.hk("https://accounts.google.com/AccountChooser"), "continue", this.window.location.href), "faa", "1");
				_.wd(this.window.location, _.kd(a.toString()));
			}
			nO() {
				if (this.Ep = !this.Ep) {
					jtd(this);
					ktd(this);
				}
			}
			V$(a) {
				a.stopPropagation();
				this.Ep = true;
				jtd(this);
				ktd(this);
				document.activeElement.addEventListener("keydown", console.log);
			}
			r5(a) {
				a.stopPropagation();
				this.Ep = false;
				this.Wfa.nativeElement.focus();
			}
		};
		Y3.J = function(a) {
			return new (a || Y3)(_.Dg(_.WK), _.Dg(_.SK), _.Dg(_.aFb), _.Dg(_.bFb, 8), _.Dg(_.Hu));
		};
		Y3.ka = _.u({
			type: Y3,
			da: [["alkali-accountswitcher"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ci(itd, 5);
				}
				if (a & 2) {
					let c;
					if (_.ei(c = _.fi())) {
						b.Wfa = c.first;
					}
				}
			},
			ha: 6,
			ia: 4,
			la: () => [
				["unauthenticated", ""],
				["avatar", ""],
				[
					"buttonContainer",
					"",
					"trigger",
					"cdkOverlayOrigin"
				],
				["monogram", ""],
				" �0� ",
				" �0� ",
				"Privacy Policy",
				"Terms of Service",
				" �0� ",
				[
					"id",
					"account-switcher-button",
					"class",
					"container",
					4,
					"ngIf",
					"ngIfElse"
				],
				[
					"id",
					"account-switcher-button",
					1,
					"container"
				],
				[
					"role",
					"button",
					"tabindex",
					"0",
					"cdkOverlayOrigin",
					"",
					1,
					"button-container",
					3,
					"click",
					"keydown.enter",
					"keydown.space",
					"keydown.escape",
					"matTooltip"
				],
				[
					4,
					"ngTemplateOutlet",
					"ngTemplateOutletContext"
				],
				[
					"cdkConnectedOverlay",
					"",
					"cdkConnectedOverlayLockPosition",
					"",
					3,
					"cdkConnectedOverlayOrigin",
					"cdkConnectedOverlayPositions",
					"cdkConnectedOverlayOpen"
				],
				[
					"cdkTrapFocus",
					"",
					"id",
					"account-switcher",
					1,
					"account-switcher",
					3,
					"keydown.escape"
				],
				[
					"id",
					"accountSwitcherFirstFocus",
					"tabindex",
					"-1"
				],
				[1, "profile"],
				[1, "name"],
				[1, "email"],
				[
					"mat-stroked-button",
					"",
					"color",
					"primary",
					1,
					"switch-account-button",
					3,
					"click"
				],
				[1, "divider"],
				[
					"mat-button",
					"",
					"color",
					"primary",
					1,
					"signout-button",
					3,
					"click"
				],
				[1, "policy"],
				[1, "policy-label"],
				[
					"id",
					"privacyPolicyLink",
					3,
					"href"
				],
				["aria-hidden", "true"],
				[
					"id",
					"tosLink",
					3,
					"href"
				],
				[
					"mat-flat-button",
					"",
					"color",
					"primary",
					1,
					"signin-button",
					3,
					"click"
				],
				[
					3,
					"size",
					"src",
					"alt",
					4,
					"ngIf",
					"ngIfElse"
				],
				[
					3,
					"size",
					"src",
					"alt"
				],
				[
					"aria-label",
					"Profile photo",
					"role",
					"img",
					3,
					"initials",
					"size"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.z(0, Qod, 6, 10, "div", 9), _.Ei(1, "async"), _.z(2, Rod, 2, 1, "ng-template", null, 0, _.Ii)(4, Uod, 3, 2, "ng-template", null, 1, _.Ii);
				}
				if (a & 2) {
					a = _.O(3), _.E("ngIf", _.Fi(1, 2, b.F3a))("ngIfElse", a);
				}
			},
			dependencies: [
				_.QK,
				_.GB,
				_.FB,
				_.JA,
				_.VC,
				_.UC,
				_.XB,
				_.XC,
				_.WC,
				_.HC,
				_.ND,
				_.RK,
				_.mz,
				_.nz,
				_.oz
			],
			styles: [".container[_ngcontent-%COMP%]{position:relative}.button-container[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;cursor:pointer;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;height:42px;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;position:relative;width:42px}.button-container[_ngcontent-%COMP%]   connect-avatar[_ngcontent-%COMP%]{height:36px;width:36px}.button-container[_ngcontent-%COMP%]   connect-monogram[_ngcontent-%COMP%]{height:36px;width:36px}.button-container[_ngcontent-%COMP%]:hover{background-color:rgba(60,64,67,.1);border-radius:50%;outline:none}.button-container[_ngcontent-%COMP%]:focus{background-color:rgba(95,99,104,.24);border-radius:50%;outline:none}.account-switcher[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;background:#fff;box-shadow:0 4px 8px 3px rgba(60,64,67,.15),0 1px 3px rgba(60,64,67,.3);border-radius:8px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;position:absolute;right:20px;width:280px}.profile[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;margin-top:12px;width:100%}.name[_ngcontent-%COMP%]{color:#3c4043;font-family:Google Sans;font-size:16px;font-weight:700;line-height:24px;letter-spacing:.1px;margin-top:12px;text-align:center;text-transform:none}.email[_ngcontent-%COMP%]{color:#5f6368;font-family:Roboto;font-size:14px;font-style:normal;font-weight:400;letter-spacing:.2px;line-height:20px;text-align:center;text-transform:none}.divider[_ngcontent-%COMP%]{background:#dadce0;width:100%}.button-label[_ngcontent-%COMP%]{color:#1a73e8;font-family:Google Sans;font-size:14px;line-height:20px;letter-spacing:.25px;margin:0 8px;text-align:center;text-transform:none}.signin-button[_ngcontent-%COMP%]{margin-right:8px}.signout-button[_ngcontent-%COMP%]{margin:2px;padding:12px}.switch-account-button[_ngcontent-%COMP%]{margin:24px;padding:8px 24px}.policy[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}.policy[_ngcontent-%COMP%]   span[_ngcontent-%COMP%]{font-weight:700;margin-bottom:12px;margin-top:12px}.policy-label[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;margin-bottom:12px;margin-top:12px;text-align:center}.policy-label[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]{border-radius:4px;color:#5f6368;font-family:Roboto;font-size:12px;font-style:normal;font-weight:400;padding:4px 8px}.policy-label[_ngcontent-%COMP%]   [_ngcontent-%COMP%]:hover{background-color:#f7f8f8;outline:none}.policy-label[_ngcontent-%COMP%]   [_ngcontent-%COMP%]:focus{background-color:#f4f4f4;outline:none}"]
		});
		var ltd = class {
			constructor() {
				this.cL = _.m(_.s3);
				this.Hhb = etd;
			}
			ib() {
				var a = this.cL;
				var b;
				var c = Number((b = a.window.localStorage.getItem("bannerVersionViewed")) != null ? b : "0");
				var d = a.A.length;
				b = -1;
				for (let e = 0; e < d; ++e) if ((a.A[e] > c || c === 0) && Wsd(a, e)) {
					b = e;
					c = a.A[e];
					if (c === 8) {
						c = "bannerFirstSeenDate_" + c.toString(), a.window.localStorage.getItem(c) || a.window.localStorage.setItem(c, Date.now().toString());
					}
					break;
				}
				a.H.set(b);
			}
			y6() {
				this.cL.y6();
			}
		};
		ltd.J = function(a) {
			return new (a || ltd)();
		};
		ltd.ka = _.u({
			type: ltd,
			da: [["ms-banner"]],
			ha: 1,
			ia: 1,
			la: [
				[1, "banner-container"],
				[1, "error-banner-message"],
				[
					"ms-button",
					"",
					1,
					"dismiss",
					"custom-theme",
					3,
					"click"
				],
				[1, "error-banner-message-text"],
				[
					"target",
					"_blank",
					"rel",
					"noopener",
					1,
					"learn-more-link",
					3,
					"href"
				],
				[
					"href",
					"https://ai.google.dev/gemini-api/terms",
					"target",
					"_blank",
					"rel",
					"noopener",
					3,
					"click"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, Zod, 5, 1, "div", 0);
				}
				if (a & 2) {
					_.C(b.cL.Aga() ? 0 : -1);
				}
			},
			dependencies: [_.Yy, _.tz],
			styles: [".base-header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;height:76px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between}.base-header[_ngcontent-%COMP%]   .left-side[_ngcontent-%COMP%], .base-header[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px}.base-header[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{margin-left:12px}@media screen and (max-width:600px){.base-header[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{gap:4px;margin-left:4px}}.dialog-header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;padding:12px 24px}.prompt-header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:2px;height:44px}.prompt-header[_ngcontent-%COMP%]   h3[_ngcontent-%COMP%]{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.prompt-header[_ngcontent-%COMP%]   .left-side[_ngcontent-%COMP%], .prompt-header[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:2px}.prompt-bar[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;bottom:0;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;z-index:3}.prompt-bar[_ngcontent-%COMP%]   .left-side[_ngcontent-%COMP%], .prompt-bar[_ngcontent-%COMP%]   .right-side[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px}.center[_ngcontent-%COMP%]{text-align:center}.loading-indicator-container[_ngcontent-%COMP%]{overflow:visible}form[_ngcontent-%COMP%]   label[_ngcontent-%COMP%]{color:var(--color-on-surface);font-weight:500}form[_ngcontent-%COMP%]   label[_ngcontent-%COMP%]   sup[_ngcontent-%COMP%]{line-height:0}form[_ngcontent-%COMP%]   label[_ngcontent-%COMP%]   mat-hint[_ngcontent-%COMP%]{display:block}form[_ngcontent-%COMP%]   mat-checkbox[_ngcontent-%COMP%]{width:100%}form[_ngcontent-%COMP%]   mat-form-field[_ngcontent-%COMP%]{color:var(--color-on-surface);max-width:425px;min-width:425px;width:100%}@media screen and (max-width:768px){form[_ngcontent-%COMP%]   mat-form-field[_ngcontent-%COMP%]{max-width:300px;min-width:300px}}@media screen and (max-width:600px){form[_ngcontent-%COMP%]   mat-form-field[_ngcontent-%COMP%]{max-width:unset;min-width:unset}}form[_ngcontent-%COMP%]   .form-row[_ngcontent-%COMP%]{-webkit-box-align:start;-webkit-align-items:start;-moz-box-align:start;-ms-flex-align:start;align-items:start;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;margin-bottom:48px}@media screen and (max-width:720px){form[_ngcontent-%COMP%]   label[_ngcontent-%COMP%]{-webkit-transform:initial;transform:none}form[_ngcontent-%COMP%]   .form-row[_ngcontent-%COMP%]{-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}}.bold[_ngcontent-%COMP%]{font-weight:700}.link-icon[_ngcontent-%COMP%]{vertical-align:sub}.banner-container[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;background-color:var(--color-v3-surface-container);border:1px solid var(--color-v3-outline-var);box-shadow:var(--v3-shadow-lg);color:var(--color-v3-text);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;max-height:60px;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:12px 24px}.banner-container[_ngcontent-%COMP%]   .learn-more-link[_ngcontent-%COMP%]{text-wrap:nowrap}.banner-container[_ngcontent-%COMP%]   p.error-banner-message[_ngcontent-%COMP%]:has(a.learn-more-link)   .error-banner-message-text[_ngcontent-%COMP%]{margin-right:8px}.banner-container[_ngcontent-%COMP%]   .dismiss[_ngcontent-%COMP%]{margin-left:8px}.banner-container[_ngcontent-%COMP%]   .dismiss.gmat-mdc-button.mat-mdc-button[_ngcontent-%COMP%]:not(.mat-mdc-button-disabled):not([disabled]):not(:disabled){color:var(--color-banner-content)}.banner-container[_ngcontent-%COMP%]   .dismiss.gmat-mdc-button.mat-mdc-button[_ngcontent-%COMP%]:not(.mat-mdc-button-disabled):not([disabled]):not(:disabled):hover{background-color:var(--color-banner-button-highlight)}"]
		});
		var Z3 = class {
			constructor() {
				this.A = _.m(_.Ou);
				this.Qc = _.m(_.BM);
				this.F = _.m(_.ZC);
				this.S = _.Dk;
				this.ve = { Mnb: 227417 };
			}
			Sf() {
				if (this.F.A.Il()) {
					this.Qc.iu(false, true);
				}
				_.Rn(this.A, "NAV", "Clicked Create API Key Button");
			}
		};
		Z3.J = function(a) {
			return new (a || Z3)();
		};
		Z3.ka = _.u({
			type: Z3,
			da: [["ms-api-key-button"]],
			ha: 2,
			ia: 4,
			la: [[
				"ms-button",
				"",
				"variant",
				"borderless",
				"routerLink",
				"apikey",
				3,
				"click",
				"iconName",
				"ve",
				"veImpression",
				"veClick"
			]],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "button", 0), _.J("click", function() {
						return b.Sf();
					}), _.R(1, " Get API key\n"), _.H();
				}
				if (a & 2) {
					_.E("iconName", b.S.ly)("ve", b.ve.Mnb)("veImpression", true)("veClick", true);
				}
			},
			dependencies: [
				_.Yy,
				_.tz,
				_.sA,
				_.Cz,
				_.Bz
			],
			styles: ["button[ms-button][_ngcontent-%COMP%]{width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;color:var(--color-v3-text-var)}button[ms-button][_ngcontent-%COMP%]   .button-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}button[ms-button][_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}"]
		});
		var mtd = new _.$y("45778471", false);
		var bpd = class {
			constructor() {
				this.Ia = _.m(_.oF);
				this.A = _.m(_.Op);
				_.m(_.zF);
				this.AM = _.Aa();
				this.VCa = _.lp();
				this.aw = htd;
				this.Wv = gtd;
				this.theme = this.Ia.theme.asReadonly();
				this.enterKeyBehavior = this.Ia.enterKeyBehavior;
				this.codeAssistantNotifications = this.Ia.codeAssistantNotifications.asReadonly();
				this.aCa = this.A.getFlag(_.CF);
				this.G3a = this.A.getFlag(mtd);
				this.autocompleteEnabled = this.Ia.autocompleteEnabled;
			}
			setTheme(a) {
				this.Ia.theme.set(a);
			}
			BJ(a) {
				this.Ia.BJ(a);
			}
			wba(a) {
				this.Ia.codeAssistantNotifications.set(a ? "granted" : "denied");
				if (a) {
					_.Lpb();
				}
			}
			tba(a) {
				Nsd(this.Ia, a);
			}
		};
		bpd.J = function(a) {
			return new (a || bpd)();
		};
		bpd.ka = _.u({
			type: bpd,
			da: [["ms-user-settings-dialog"]],
			ha: 31,
			ia: 7,
			la: [
				["mat-dialog-title", ""],
				[1, "settings-content"],
				[1, "setting-section"],
				[1, "setting-label"],
				[1, "setting-description"],
				[1, "button-group"],
				[
					"ms-button",
					"",
					"variant",
					"filter-chip",
					3,
					"click",
					"active"
				],
				[
					"data-test-id",
					"notifications-section",
					1,
					"setting-section"
				],
				"ms-button  variant primary mat-dialog-close ".split(" "),
				[
					"ms-button",
					"",
					"variant",
					"filter-chip",
					"data-test-id",
					"submit-key-newline",
					3,
					"click",
					"active"
				],
				[
					"ms-button",
					"",
					"variant",
					"filter-chip",
					"data-test-id",
					"submit-key-submit",
					3,
					"click",
					"active"
				],
				[
					"data-test-id",
					"autocomplete-section",
					1,
					"setting-section"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "h2", 0), _.R(1, "User Settings"), _.H(), _.F(2, "mat-dialog-content", 1)(3, "div", 2)(4, "div", 3), _.R(5, "Theme"), _.H(), _.F(6, "div", 4), _.R(7, " Adjust the visual appearance. "), _.H(), _.F(8, "div", 5)(9, "button", 6), _.J("click", function() {
						return b.setTheme(b.aw.sea);
					}), _.R(10, " Light "), _.H(), _.F(11, "button", 6), _.J("click", function() {
						return b.setTheme(b.aw.Yda);
					}), _.R(12, " Dark "), _.H(), _.F(13, "button", 6), _.J("click", function() {
						return b.setTheme(b.aw.SYSTEM);
					}), _.R(14, " System "), _.H()()(), _.B(15, $od, 11, 3), _.I(16, "mat-divider"), _.B(17, apd, 11, 2), _.F(18, "div", 7)(19, "div", 3), _.R(20, "Applet notifications"), _.H(), _.F(21, "div", 4), _.R(22, " Receive browser notifications when Build operations complete. "), _.H(), _.F(23, "div", 5)(24, "button", 6), _.J("click", function() {
						return b.wba(true);
					}), _.R(25, " Enabled "), _.H(), _.F(26, "button", 6), _.J("click", function() {
						return b.wba(false);
					}), _.R(27, " Disabled "), _.H()()()(), _.F(28, "mat-dialog-actions")(29, "button", 8), _.R(30, "Close"), _.H()();
				}
				if (a & 2) {
					_.y(9), _.E("active", b.theme() === b.aw.sea), _.y(2), _.E("active", b.theme() === b.aw.Yda), _.y(2), _.E("active", b.theme() === b.aw.SYSTEM), _.y(2), _.C(b.VCa ? -1 : 15), _.y(2), _.C(b.aCa ? 17 : -1), _.y(7), _.E("active", b.codeAssistantNotifications() === "granted"), _.y(2), _.E("active", b.codeAssistantNotifications() !== "granted");
				}
			},
			dependencies: [
				_.xC,
				_.sC,
				_.uC,
				_.wC,
				_.vC,
				_.OD,
				_.ND,
				_.Yy
			],
			styles: ["[_nghost-%COMP%]{--mat-dialog-headline-padding:0 16px}.settings-content[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:16px}.setting-section[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px;padding:8px 0}.setting-label[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:16px;font-weight:600;line-height:24px;font-weight:500}.button-group[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:8px}.setting-description[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-var);margin-bottom:4px}"]
		});
		var ntd = new _.$y("45736286", false);
		var $3 = class {
			constructor() {
				this.S = _.Dk;
				this.A = _.m(_.Op);
				this.Ia = _.m(_.oF);
				this.Dza = _.m(_.t3);
				this.F = _.m(_.uG);
				_.m(_.zF);
				this.dialog = _.m(_.rC);
				this.G3a = this.A.getFlag(mtd);
				this.AM = _.Aa();
				this.VCa = _.lp();
				this.Bhb = "https://cloud.google.com/support/billing";
				this.QOa = "/gemini-api/terms";
				this.qG = "https://policies.google.com/privacy";
				this.aw = htd;
				this.Wv = gtd;
				this.ve = {
					Onb: 227591,
					brb: 312514,
					crb: 304182,
					drb: 304183,
					erb: 276231,
					frb: 276230,
					wrb: 227593,
					xrb: 227592,
					yrb: 227594
				};
				this.theme = this.Ia.theme.asReadonly();
				this.enterKeyBehavior = this.Ia.enterKeyBehavior;
				this.codeAssistantNotifications = this.Ia.codeAssistantNotifications.asReadonly();
				this.fIb = this.A.getFlag(ntd);
				this.aCa = this.A.getFlag(_.CF);
				this.autocompleteEnabled = this.Ia.autocompleteEnabled;
				this.Qvb = _.W(() => this.F.F() ? "/gemini-api/terms#unpaid-services" : "/gemini-api/terms#paid-services");
				this.Rvb = _.W(() => this.F.F() ? "Unpaid services ToS" : "Paid services ToS");
			}
			setTheme(a) {
				this.Ia.theme.set(a);
			}
			BJ(a) {
				this.Ia.BJ(a);
			}
			wba(a) {
				this.Ia.codeAssistantNotifications.set(a ? "granted" : "denied");
				if (a) {
					_.Lpb();
				}
			}
			tba(a) {
				Nsd(this.Ia, a);
			}
		};
		$3.J = function(a) {
			return new (a || $3)();
		};
		$3.ka = _.u({
			type: $3,
			da: [["ms-settings-menu"]],
			ha: 79,
			ia: 69,
			la: [
				["settingsMenu", ""],
				["themeMenu", ""],
				["submitPromptKeyMenu", ""],
				["codeAssistantNotificationsMenu", ""],
				["accountStatusMenu", ""],
				["autocompleteMenu", ""],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"data-test-id",
					"settings-menu",
					1,
					"trigger-button",
					3,
					"iconName",
					"matMenuTriggerFor",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"mat-menu-item",
					"",
					"data-test-id",
					"user-settings-menu"
				],
				["aria-hidden", "true"],
				[
					"mat-menu-item",
					"",
					"data-test-id",
					"account-status-menu",
					3,
					"matMenuTriggerFor"
				],
				"mat-menu-item;;href;https://aistudio.google.com/status;target;_blank;rel;noopener noreferrer;aria-label;View AI Studio and Gemini status page".split(";"),
				[3, "iconName"],
				[
					"mat-menu-item",
					"",
					3,
					"documentation-path"
				],
				[
					"mat-menu-item",
					"",
					"target",
					"_blank",
					"rel",
					"noreferrer",
					3,
					"href"
				],
				[
					"mat-menu-item",
					"",
					"xapFeedback",
					"",
					3,
					"xapFeedbackConfig"
				],
				[
					"mat-menu-item",
					"",
					"target",
					"_blank",
					"rel",
					"noreferrer",
					3,
					"href",
					"ve",
					"veImpression",
					"veClick"
				],
				["xPosition", "after"],
				[
					"mat-menu-item",
					"",
					"role",
					"menuitemradio",
					"aria-label",
					"Enable light theme",
					3,
					"click",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"aria-hidden",
					"true",
					1,
					"radio-dot"
				],
				[
					"mat-menu-item",
					"",
					"role",
					"menuitemradio",
					"aria-label",
					"Enable dark theme",
					3,
					"click",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"mat-menu-item",
					"",
					"role",
					"menuitemradio",
					"aria-label",
					"Enable system theme",
					3,
					"click",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"mat-menu-item",
					"",
					"role",
					"menuitemradio",
					"matTooltipClass",
					"submit-prompt-key-tooltip",
					"matTooltipPosition",
					"right",
					3,
					"click",
					"matTooltip",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"mat-menu-item",
					"",
					"role",
					"menuitemradio",
					"matTooltip",
					"Enable applet notifications",
					"matTooltipClass",
					"submit-prompt-key-tooltip",
					"matTooltipPosition",
					"right",
					3,
					"click",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"mat-menu-item",
					"",
					"role",
					"menuitemradio",
					"matTooltip",
					"Disable applet notifications",
					"matTooltipClass",
					"submit-prompt-key-tooltip",
					"matTooltipPosition",
					"right",
					3,
					"click",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"mat-menu-item",
					"",
					"role",
					"menuitemradio",
					"matTooltip",
					"Enable autocomplete suggestions while typing",
					"matTooltipClass",
					"submit-prompt-key-tooltip",
					"matTooltipPosition",
					"right",
					"data-test-id",
					"autocomplete-enabled",
					3,
					"click"
				],
				[
					"mat-menu-item",
					"",
					"role",
					"menuitemradio",
					"matTooltip",
					"Disable autocomplete suggestions",
					"matTooltipClass",
					"submit-prompt-key-tooltip",
					"matTooltipPosition",
					"right",
					"data-test-id",
					"autocomplete-disabled",
					3,
					"click"
				],
				[
					"mat-menu-item",
					"",
					"data-test-id",
					"user-settings-menu",
					3,
					"click"
				],
				[
					"mat-menu-item",
					"",
					"data-test-id",
					"theme-menu",
					3,
					"matMenuTriggerFor"
				],
				[
					"mat-menu-item",
					"",
					3,
					"matMenuTriggerFor"
				],
				[
					"mat-menu-item",
					"",
					"data-test-id",
					"autocomplete-menu",
					3,
					"matMenuTriggerFor"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					let c = _.n();
					_.F(0, "button", 6);
					_.R(1, " Settings\n");
					_.H();
					_.F(2, "mat-menu", null, 0);
					_.B(4, cpd, 3, 1, "button", 7)(5, fpd, 8, 6);
					_.I(6, "mat-divider", 8);
					_.B(7, gpd, 3, 2, "button", 9);
					_.F(8, "a", 10);
					_.I(9, "span", 11);
					_.R(10, " View status ");
					_.H();
					_.F(11, "a", 12);
					_.I(12, "span", 11);
					_.F(13, "span");
					_.R(14, "Terms of service");
					_.H()();
					_.F(15, "a", 13);
					_.I(16, "span", 11);
					_.F(17, "span");
					_.R(18, "Privacy policy");
					_.H()();
					_.F(19, "button", 14);
					_.Ei(20, "async");
					_.I(21, "span", 11);
					_.F(22, "span");
					_.R(23, "Send feedback");
					_.H()();
					_.F(24, "a", 15);
					_.I(25, "span", 11);
					_.F(26, "span");
					_.R(27, "Billing Support");
					_.H()()();
					_.F(28, "mat-menu", 16, 1)(30, "button", 17);
					_.J("click", function(d) {
						_.q(c);
						b.setTheme(b.aw.sea);
						return _.t(d.stopPropagation());
					});
					_.I(31, "span", 18)(32, "span", 11);
					_.R(33, " Light ");
					_.H();
					_.F(34, "button", 19);
					_.J("click", function(d) {
						_.q(c);
						b.setTheme(b.aw.Yda);
						return _.t(d.stopPropagation());
					});
					_.I(35, "span", 18)(36, "span", 11);
					_.R(37, " Dark ");
					_.H();
					_.F(38, "button", 20);
					_.J("click", function(d) {
						_.q(c);
						b.setTheme(b.aw.SYSTEM);
						return _.t(d.stopPropagation());
					});
					_.I(39, "span", 18)(40, "span", 11);
					_.R(41, " System ");
					_.H()();
					_.F(42, "mat-menu", 16, 2)(44, "button", 21);
					_.J("click", function(d) {
						_.q(c);
						b.BJ(b.Wv.dea);
						return _.t(d.stopPropagation());
					});
					_.I(45, "span", 18);
					_.F(46, "span");
					_.B(47, hpd, 1, 0)(48, ipd, 1, 0);
					_.H()();
					_.F(49, "button", 21);
					_.J("click", function(d) {
						_.q(c);
						b.BJ(b.Wv.eea);
						return _.t(d.stopPropagation());
					});
					_.I(50, "span", 18);
					_.F(51, "span");
					_.R(52, "Enter");
					_.H()()();
					_.F(53, "mat-menu", 16, 3)(55, "button", 22);
					_.J("click", function(d) {
						_.q(c);
						b.wba(true);
						return _.t(d.stopPropagation());
					});
					_.I(56, "span", 18);
					_.F(57, "span");
					_.R(58, "Enabled");
					_.H()();
					_.F(59, "button", 23);
					_.J("click", function(d) {
						_.q(c);
						b.wba(false);
						return _.t(d.stopPropagation());
					});
					_.I(60, "span", 18);
					_.F(61, "span");
					_.R(62, "Disabled");
					_.H()()();
					_.F(63, "mat-menu", 16, 4)(65, "a", 12)(66, "span");
					_.R(67);
					_.H();
					_.I(68, "span", 11);
					_.H()();
					_.F(69, "mat-menu", 16, 5)(71, "button", 24);
					_.J("click", function(d) {
						_.q(c);
						b.tba(true);
						return _.t(d.stopPropagation());
					});
					_.I(72, "span", 18);
					_.F(73, "span");
					_.R(74, "Enabled");
					_.H()();
					_.F(75, "button", 25);
					_.J("click", function(d) {
						_.q(c);
						b.tba(false);
						return _.t(d.stopPropagation());
					});
					_.I(76, "span", 18);
					_.F(77, "span");
					_.R(78, "Disabled");
					_.H()()();
				}
				if (a & 2) {
					a = _.O(3);
					_.E("iconName", b.S.Vs)("matMenuTriggerFor", a)("ve", b.ve.Onb)("veImpression", true)("veClick", true);
					_.y(4);
					_.C(b.G3a ? 4 : 5);
					_.y(3);
					_.C(b.fIb ? 7 : -1);
					_.y(2);
					_.E("iconName", b.S.Cjb);
					_.y(2);
					_.E("documentation-path", b.QOa);
					_.y();
					_.E("iconName", b.S.j2);
					_.y(3);
					_.E("href", b.qG, _.rg);
					_.y();
					_.E("iconName", b.S.jqb);
					_.y(3);
					let c;
					_.E("xapFeedbackConfig", (c = _.Fi(20, 67, b.Dza.A5)) != null ? c : undefined);
					_.y(2);
					_.E("iconName", b.S.w2);
					_.y(3);
					_.E("href", b.Bhb, _.rg)("ve", b.ve.brb)("veImpression", true)("veClick", true);
					_.y();
					_.E("iconName", b.S.Nmb);
					_.y(5);
					_.E("ve", b.ve.xrb)("veImpression", true)("veClick", true);
					_.y();
					_.P("selected", b.theme() === b.aw.sea);
					_.y();
					_.E("iconName", b.S.emb);
					_.y(2);
					_.E("ve", b.ve.wrb)("veImpression", true)("veClick", true);
					_.y();
					_.P("selected", b.theme() === b.aw.Yda);
					_.y();
					_.E("iconName", b.S.Anb);
					_.y(2);
					_.E("ve", b.ve.yrb)("veImpression", true)("veClick", true);
					_.y();
					_.P("selected", b.theme() === b.aw.SYSTEM);
					_.y();
					_.E("iconName", b.S.Sib);
					_.y(4);
					_.E("matTooltip", "Submit: " + (b.AM ? "Cmd" : "Ctrl") + " + Enter\nNewline: Enter")("ve", b.ve.erb)("veImpression", true)("veClick", true);
					_.y();
					_.P("selected", b.enterKeyBehavior() === b.Wv.dea);
					_.y(2);
					_.C(b.AM ? 47 : 48);
					_.y(2);
					_.E("matTooltip", "Submit: Enter\nNewline: Shift + Enter")("ve", b.ve.frb)("veImpression", true)("veClick", true);
					_.y();
					_.P("selected", b.enterKeyBehavior() === b.Wv.eea);
					_.y(5);
					_.E("ve", b.ve.drb)("veImpression", true)("veClick", true);
					_.y();
					_.P("selected", b.codeAssistantNotifications() === "granted");
					_.y(3);
					_.E("ve", b.ve.crb)("veImpression", true)("veClick", true);
					_.y();
					_.P("selected", b.codeAssistantNotifications() === "denied");
					_.y(5);
					_.E("documentation-path", b.Qvb());
					_.y(2);
					_.U(b.Rvb());
					_.y();
					_.E("iconName", b.S.Ps);
					_.y(4);
					_.P("selected", b.autocompleteEnabled());
					_.y(4);
					_.P("selected", !b.autocompleteEnabled());
				}
			},
			dependencies: [
				_.Yy,
				_.LC,
				_.v3,
				_.dz,
				_.OD,
				_.ND,
				_.wI,
				_.tI,
				_.sI,
				_.vI,
				_.IC,
				_.HC,
				_.Cz,
				_.Bz,
				_.oz
			],
			styles: [".trigger-button[_ngcontent-%COMP%]{width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;color:var(--color-v3-text-var)}.trigger-button[_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.radio-dot[_ngcontent-%COMP%]{width:8px;height:8px;border:1px solid var(--mat-menu-item-label-text-color);border-radius:50%;display:inline-block;vertical-align:middle;margin-right:14px}.radio-dot.selected[_ngcontent-%COMP%]{background-color:var(--mat-menu-item-label-text-color)}"]
		});
		var otd = class {
			transform(a, b) {
				if (!a) return b;
				var c;
				return (c = _.Ek(a)) != null ? c : b;
			}
		};
		otd.J = function(a) {
			return new (a || otd)();
		};
		otd.Wo = _.Xe({
			name: "toIconWithDefault",
			type: otd,
			wk: true
		});
		var qtd = function(a) {
			_.x(function* () {
				var b = (yield Xsd(a.F)).filter((c) => _.Lm(c, 8) === 1);
				a.N7a.set(b);
			});
		};
		var a4 = class {
			constructor() {
				this.S = _.Dk;
				this.Ga = _.m(_.Jf);
				this.F = _.m(_.A3);
				this.Tc = _.M(false);
				this.N7a = _.M([]);
				this.cMa = _.hi();
				this.vka = false;
				this.lN = [{
					Lb: "end",
					Mb: "bottom",
					Bb: "start",
					Gb: "bottom",
					offsetX: 8
				}, {
					Lb: "start",
					Mb: "top",
					Bb: "start",
					Gb: "bottom",
					offsetY: -8
				}];
				this.ve = {
					Zkb: 305470,
					alb: 305471,
					blb: 305472
				};
				qtd(this);
				_.Fk([this.Tc], (a) => {
					if (this.Tc()) {
						let b = (d) => {
							d = d.relatedTarget;
							var e;
							if (!(!d || this.Ga.nativeElement.contains(d) || ((e = d.closest) == null ? 0 : e.call(d, ".cdk-overlay-container")))) {
								this.close();
							}
						};
						let c = (d) => {
							d = d.target;
							var e;
							if (!(this.Ga.nativeElement.contains(d) || ((e = d.closest) == null ? 0 : e.call(d, ".cdk-overlay-container")))) {
								this.close();
							}
						};
						document.addEventListener("focusout", b);
						document.addEventListener("click", c);
						a(() => {
							document.removeEventListener("focusout", b);
							document.removeEventListener("click", c);
							this.Nr();
						});
					}
				});
			}
			open() {
				this.Nr();
				this.Tc.set(true);
			}
			close() {
				this.vka = true;
				this.Tc.set(false);
				setTimeout(() => {
					this.vka = false;
				});
			}
			QU() {
				this.Nr();
				this.A = setTimeout(() => {
					this.close();
				}, 500);
			}
			Nr() {
				if (this.A) {
					clearTimeout(this.A), this.A = undefined;
				}
			}
			GFa(a) {
				var b = this.F;
				a = _.Z(a, Hsd, 5);
				var c = a == null ? undefined : _.fj(a, Gsd, 2, Isd);
				a = a == null ? undefined : _.qj(a, 1, Isd);
				if (c) {
					let k = c == null ? undefined : _.l(c, 1);
					let p = c == null ? undefined : c.getPrompt();
					if (p) {
						c = _.vq(p);
						var d = c == null ? undefined : c.getModel();
						var e;
						var f;
						var g = (f = (e = _.oq(p)) == null ? undefined : _.pq(e)[0]) == null ? undefined : f.getText();
						if (d) {
							e = _.AF(b.A, d), e = _.Fn(_.dya(new _.Tm().setModel(d), e)), c = Object.assign({}, e, _.Fn(c)), _.Kmd(b.F, [c]);
						}
						b.H.navigate(["prompts", "new_chat"], { queryParams: {
							model: d,
							prompt: g
						} });
					} else {
						if (k) {
							_.br(b.H, k);
						}
						if (e = c == null ? undefined : (g = c.getPrompt()) == null ? undefined : _.vq(g)) {
							if (e = e.getModel()) {
								f = _.AF(b.A, e);
								e = _.dya(new _.Tm().setModel(e), f);
								_.RC(b.F.A(), _.Fn(e));
							}
						}
						if (c = c == null ? undefined : (d = c.getPrompt()) == null ? undefined : _.oq(d)) {
							_.eK(b.I, { chunk: {
								id: "text-input-chunk-id",
								text: _.pq(c)[0].getText()
							} });
						}
					}
				}
				if (a) {
					_.rd(b.window, _.jd(a), "_blank");
				}
				this.close();
			}
			rE(a) {
				var b = this.cMa();
				if (b.length !== 0) {
					var c = document.activeElement;
					var d = b.findIndex((e) => e.nativeElement === c);
					if (a.key === "ArrowDown") {
						a.preventDefault();
						b[Math.min(b.length - 1, d + 1)].nativeElement.focus();
					} else if (a.key === "ArrowUp") {
						a.preventDefault();
						b[d === -1 ? 0 : Math.max(0, d - 1)].nativeElement.focus();
					} else if (a.key === "Escape") {
						let e;
						if (!((e = this.Ga.nativeElement.querySelector("button")) == null)) {
							e.focus();
						}
						this.close();
					}
				}
			}
		};
		a4.J = function(a) {
			return new (a || a4)();
		};
		a4.ka = _.u({
			type: a4,
			da: [["ms-updates"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.cMa, ptd, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			ha: 4,
			ia: 10,
			la: [
				["trigger", "cdkOverlayOrigin"],
				["updatesPopover", ""],
				["updateCard", ""],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"aria-label",
					"What's new",
					"aria-haspopup",
					"dialog",
					"aria-controls",
					"updates-popover",
					"cdkOverlayOrigin",
					"",
					3,
					"click",
					"keydown",
					"mouseenter",
					"mouseleave",
					"focus",
					"iconName",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"cdkConnectedOverlay",
					"",
					3,
					"cdkConnectedOverlayOrigin",
					"cdkConnectedOverlayOpen",
					"cdkConnectedOverlayPositions",
					"cdkConnectedOverlayHasBackdrop",
					"cdkConnectedOverlayPush"
				],
				[
					"id",
					"updates-popover",
					"role",
					"dialog",
					"aria-label",
					"Updates",
					"aria-modal",
					"true",
					"tabindex",
					"-1",
					1,
					"updates-popover",
					3,
					"keydown",
					"mouseenter",
					"mouseleave",
					"ve",
					"veImpression"
				],
				[
					1,
					"update-card",
					3,
					"ve",
					"veClick",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[1, "empty-state"],
				[
					1,
					"update-card",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[1, "icon-container"],
				[
					1,
					"icon",
					3,
					"iconName"
				],
				[1, "update-content"],
				[1, "update-header"],
				[1, "update-description"],
				[
					"ms-button",
					"",
					"size",
					"small",
					"variant",
					"primary",
					"tabindex",
					"-1",
					"aria-hidden",
					"true",
					"isIconPositionEnd",
					"",
					1,
					"cta-button",
					3,
					"iconName"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "button", 3, 0), _.J("click", function() {
						if (b.Tc()) {
							b.close();
						} else {
							b.open();
						}
					})("keydown", function(c) {
						return rtd(b, c);
					})("mouseenter", function() {
						return b.open();
					})("mouseleave", function() {
						return b.QU();
					})("focus", function() {
						if (b.vka) {
							b.vka = false;
						} else {
							b.open();
						}
					}), _.R(2, " What's new\n"), _.H(), _.z(3, npd, 5, 4, "ng-template", 4);
				}
				if (a & 2) {
					a = _.O(1), _.E("iconName", b.S.Bea)("ve", b.ve.Zkb)("veClick", true)("veImpression", true), _.wh("aria-expanded", b.Tc()), _.y(3), _.E("cdkConnectedOverlayOrigin", a)("cdkConnectedOverlayOpen", b.Tc())("cdkConnectedOverlayPositions", b.lN)("cdkConnectedOverlayHasBackdrop", false)("cdkConnectedOverlayPush", true);
				}
			},
			dependencies: [
				_.TA,
				_.Yy,
				_.dz,
				_.HB,
				_.GB,
				_.FB,
				_.Cz,
				_.Bz,
				_.hz,
				otd
			],
			styles: ["button[ms-button][_ngcontent-%COMP%]{width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;color:var(--color-v3-text-var)}button[ms-button][_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.updates-popover[_ngcontent-%COMP%]{z-index:1101;background:var(--color-v3-surface-container);border:1px solid var(--color-v3-outline-var);border-radius:12px;box-shadow:var(--v3-shadow-lg);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;width:360px;max-width:calc(100vw - 32px);max-height:400px;overflow-y:auto;padding:8px}.update-card[_ngcontent-%COMP%]{background:none;border:none;width:100%;text-align:left;font-family:inherit;color:inherit;cursor:pointer;border-radius:8px;padding:12px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:12px;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;-webkit-transition:background-color .1s ease;transition:background-color .1s ease}.update-card[_ngcontent-%COMP%]:hover{background:var(--color-v3-hover)}.update-card[_ngcontent-%COMP%]:hover   .icon-container[_ngcontent-%COMP%]{background:var(--color-v3-button-container-high);border-color:var(--color-v3-outline)}.update-card[_ngcontent-%COMP%]:hover   .icon[_ngcontent-%COMP%]{color:var(--color-v3-text)}.update-card[_ngcontent-%COMP%]:focus-visible{background:var(--color-v3-button-container-high);outline:none}.update-card[_ngcontent-%COMP%]:focus-visible   .icon-container[_ngcontent-%COMP%]{background:var(--color-v3-button-container-highest);border-color:var(--color-v3-outline)}.icon-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:42px;height:42px;border-radius:8px;border:1px solid var(--color-v3-outline-var);background:var(--color-v3-surface-container-high);-webkit-transition:all .2s ease;transition:all .2s ease;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.icon-container[_ngcontent-%COMP%]   .icon[_ngcontent-%COMP%]{font-size:20px;color:var(--color-v3-text-var)}.update-content[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;min-width:0}.update-header[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px}.update-description[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var)}.cta-button[_ngcontent-%COMP%]{margin-top:8px;width:-webkit-fit-content;width:-moz-fit-content;width:fit-content;pointer-events:none}.empty-state[_ngcontent-%COMP%]{padding:16px;text-align:center;color:var(--color-v3-text-var)}"],
			data: { animation: [_.qm("popover", [_.um(":enter", [_.sm({
				opacity: 0,
				transform: "translateX(-8px)"
			}), _.rm("200ms cubic-bezier(0.2, 0, 0, 1)", _.sm({
				opacity: 1,
				transform: "translateX(0)"
			}))]), _.um(":leave", [_.rm("150ms cubic-bezier(0.4, 0, 1, 1)", _.sm({
				opacity: 0,
				transform: "translateX(-8px)"
			}))])])] }
		});
		var std = [_.qm("parent", [_.um(":enter", [])]), _.qm("fadeInOut", [_.um(":enter", [_.sm({ opacity: 0 }), _.rm("200ms ease-in", _.sm({ opacity: 1 }))]), _.um(":leave", [_.rm("200ms ease-in", _.sm({ opacity: 0 }))])])];
		var ytd = function(a, b) {
			if (_.w3(b)) {
				a = a.N_a();
			} else {
				b === "models/med-gemini" ? a = a.M_a() : a = (b = _.AF(a.H, b)) && _.Lm(b, 23) === 1 ? a.A7a() : a.lS ? undefined : a.aYa();
			}
			return a;
		};
		var b4 = class {
			constructor() {
				this.I = _.m(_.pG);
				this.H = _.m(_.BF);
				this.A = _.m(_.gH);
				this.isNavbarExpanded = _.V(false);
				this.S = _.Dk;
				this.F = _.m(_.Op);
				this.lS = this.F.getFlag(_.bob);
				this.xn = this.I.url;
				this.N_a = _.Ni.required("gemmaDisclaimer");
				this.M_a = _.Ni.required("geminiMedDisclaimer");
				this.A7a = _.Ni.required("previewDisclaimer");
				this.XSb = _.Ni.required("syncIdDisclaimer");
				this.aYa = _.Ni.required("defaultDisclaimer");
				this.ZPb = _.W(() => {
					var a = this.xn();
					return (a == null ? undefined : a.startsWith("/prompts")) || (a == null ? undefined : a.startsWith("/live")) || (a == null ? undefined : a.startsWith("/apps"));
				});
				this.F4a = _.W(() => {
					var a;
					var b = (a = this.A.F()[0].model()) != null ? a : "";
					var c;
					return ((c = this.xn()) == null ? 0 : c.startsWith("/live")) ? this.lS ? undefined : ytd(this, "") : ytd(this, b);
				});
				this.OLb = _.W(() => {
					var a;
					var b = (a = this.A.F()[1]) == null ? undefined : a.model();
					if (b && (a = ytd(this, b), a !== this.F4a())) return a;
				});
			}
		};
		b4.J = function(a) {
			return new (a || b4)();
		};
		b4.ka = _.u({
			type: b4,
			da: [["ms-navbar-disclaimer"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.N_a, ttd, 5)(b.M_a, utd, 5)(b.A7a, vtd, 5)(b.XSb, wtd, 5)(b.aYa, xtd, 5);
				}
				if (a & 2) {
					_.ki(5);
				}
			},
			inputs: { isNavbarExpanded: [1, "isNavbarExpanded"] },
			ha: 13,
			ia: 1,
			la: [
				["disclaimer", ""],
				["gemmaDisclaimer", ""],
				["geminiMedDisclaimer", ""],
				["previewDisclaimer", ""],
				["defaultDisclaimer", ""],
				["disclaimerDialog", ""],
				[
					1,
					"disclaimer-container",
					3,
					"collapsed"
				],
				[1, "disclaimer-container"],
				[1, "disclaimer"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Disclaimer",
					"dialogLabel",
					"disclaimer",
					"matTooltip",
					"Disclaimer",
					"matTooltipPosition",
					"right",
					1,
					"disclaimer-trigger",
					3,
					"iconName",
					"xapInlineDialog"
				],
				[3, "ngTemplateOutlet"],
				[1, "separator"],
				[
					"href",
					"https://ai.google.dev/gemma",
					"target",
					"_blank"
				],
				[
					"href",
					"https://docs.google.com/document/d/1RFIwl7au8M1Xlhovsbvk33Db2UfJOSEZ",
					"target",
					"blank"
				],
				[
					"href",
					"https://ai.google.dev/gemini-api/docs/models#model-versions",
					"target",
					"blank"
				],
				[1, "disclaimer-dialog"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, qpd, 3, 3, "div", 6), _.z(1, tpd, 2, 2, "ng-template", null, 0, _.Ii)(3, vpd, 1, 1, "ng-template", null, 1, _.Ii)(5, wpd, 4, 0, "ng-template", null, 2, _.Ii)(7, xpd, 4, 0, "ng-template", null, 3, _.Ii)(9, ypd, 1, 0, "ng-template", null, 4, _.Ii)(11, zpd, 2, 1, "ng-template", null, 5, _.Ii);
				}
				if (a & 2) {
					_.C(b.ZPb() ? 0 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.tz,
				_.nz,
				_.EC
			],
			styles: [".disclaimer-container[_ngcontent-%COMP%]{-webkit-align-content:end;-ms-flex-line-pack:end;align-content:end;min-height:36px;position:relative}.disclaimer-container[_ngcontent-%COMP%]   .disclaimer[_ngcontent-%COMP%]{color:var(--color-on-surface-variant);font-size:11px;overflow-x:hidden;padding-inline:8px;width:184px}.disclaimer-container[_ngcontent-%COMP%]   .disclaimer[_ngcontent-%COMP%]   .separator[_ngcontent-%COMP%]{margin:4px 0;border-top:1px solid var(--color-inverse-on-surface)}.disclaimer-container[_ngcontent-%COMP%]   .disclaimer-trigger[_ngcontent-%COMP%]{bottom:0;height:36px;margin:0 4px;padding:6px;position:absolute;width:36px}.disclaimer-container[_ngcontent-%COMP%]   .disclaimer-trigger[_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.disclaimer-dialog[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;min-height:64px;padding:8px}"],
			data: { animation: std }
		});
		var X3 = function(a, b, c = true) {
			if (a.aa() && c) {
				a.Qc.iu(false, true);
			}
			_.Rn(a.R, "NAV", "Clicked Nav Item", b);
		};
		var c4 = class {
			constructor() {
				this.U = _.m(_.gH);
				this.H = _.m(_.pG);
				this.Ii = _.m(_.GJ);
				this.R = _.m(_.Ou);
				this.X = _.m(_.Op);
				this.ea = _.m(_.ZC);
				this.Qc = _.m(_.BM);
				this.A = _.M(null);
				this.aa = this.ea.A.ey;
				this.gY = this.X.getFlag(_.Wzb);
				this.I = _.W(() => {
					var a;
					var b;
					var c = (a = this.H.F()) == null ? undefined : (b = a.url[0]) == null ? undefined : b.path;
					return c && this.F.some((d) => d.routerLink === c) ? "dashboard" : "studio";
				});
				this.rb = _.m(_.Qp);
				_.W(() => {
					var a = this.Ii.af;
					var b = this.rb.af;
					return a() === 0 ? b() : a();
				});
				this.Lh = this.U.Lh;
				this.navItems = _.W(() => {
					switch (this.I()) {
						case "dashboard": return this.F;
						default: return this.fa;
					}
				});
				this.fa = [
					{
						text: "Chat",
						icon: "chat_spark",
						routerLink: "prompts/new_chat",
						ve: 227418
					},
					{
						text: "Stream",
						icon: "graphic_eq",
						routerLink: "live",
						ve: 240439
					},
					{
						text: "Build",
						icon: "extension",
						routerLink: "apps",
						ve: 247435
					}
				];
				this.F = [
					Atd,
					Btd,
					Ctd,
					Dtd,
					Etd,
					Ftd,
					Gtd,
					ztd,
					..._.eEb ? _.fEb : []
				];
				_.W(() => [
					{
						text: "Studio",
						url: "prompts/new_chat"
					},
					{
						text: "Dashboard",
						url: this.H.I()
					},
					{
						text: "Documentation",
						url: this.gY ? "documentation" : "https://ai.google.dev/gemini-api/docs",
						isExternal: !this.gY
					}
				]);
				_.W(() => {
					switch (this.I()) {
						case "dashboard": return "Dashboard";
						default: return "Studio";
					}
				});
			}
		};
		c4.J = function(a) {
			return new (a || c4)();
		};
		c4.sa = _.Cd({
			token: c4,
			factory: c4.J,
			wa: "root"
		});
		var Itd = class {
			constructor() {
				this.S = _.Dk;
				this.applets = _.m(_.nI);
				this.gPa = _.m(c4);
				this.A = _.m(_.ll);
				this.queryParams = _.Ck(this.A.queryParams);
				this.ve = { MNa: 282216 };
				this.ky = "Untitled";
				this.XOb = _.W(() => {
					var a;
					var b = (a = this.queryParams()) == null ? undefined : a.source;
					return Jpd.find((c) => {
						var d;
						return (c = (d = c.queryParams) == null ? undefined : d.source) || b ? c === b : true;
					});
				});
				this.F = _.W(() => this.applets.Cm.xc() ? this.applets.Cm.value() : []);
				this.Ye = _.E3;
				this.navItems = Jpd;
				this.pinnedApplets = _.W(() => this.F().filter((a) => _.Pm(a, 6)).slice(0, 10));
				this.YLa = _.W(() => {
					var a = this.pinnedApplets();
					return this.F().filter((b) => !_.Pm(b, 6)).slice(0, Math.max(0, 10 - a.length));
				});
				this.applets.ta.set(true);
				this.applets.Cm.reload();
			}
			v7(a) {
				return _.r3(a.Tf());
			}
			w7(a) {
				return _.wmd(a.Tf());
			}
			QF(a) {
				_.xmd(this.applets, a);
			}
			uB(a) {
				var b = a.getName();
				return `${_.Pm(a, 6) ? "Unpin" : "Pin"} app "${b}"`;
			}
			Bja(a) {
				return _.Xo(a.Tf());
			}
		};
		Itd.J = function(a) {
			return new (a || Itd)();
		};
		Itd.ka = _.u({
			type: Itd,
			da: [["ms-nav-items-console"]],
			ha: 9,
			ia: 6,
			la: [
				[1, "pinned-applets-container"],
				[1, "recently-viewed-applets-container"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"routerLink",
					"queryParams",
					"active",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"routerLink",
					"queryParams",
					"active"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"click",
					"routerLink",
					"queryParams",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"click",
					"routerLink",
					"queryParams"
				],
				[1, "pinned-applets-header"],
				[1, "gmat-label-large"],
				[1, "applets-list"],
				[
					1,
					"applet-link",
					3,
					"routerLink",
					"queryParams",
					"routerLinkActive",
					"routerLinkActiveOptions"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"matTooltip",
					"Unpin app",
					1,
					"pin-button",
					"pinned",
					3,
					"click",
					"aria-label",
					"variant",
					"iconName",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"matTooltip",
					"Pin app",
					1,
					"pin-button",
					3,
					"click",
					"aria-label",
					"variant",
					"iconName",
					"ve",
					"veClick",
					"veImpression"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Th(0), _.F(1, "ul"), _.Ah(2, Dpd, 3, 1, "li", null, Htd), _.H(), _.Th(4)(5), _.B(6, Epd, 1, 0, "mat-divider"), _.B(7, Gpd, 7, 0, "div", 0), _.B(8, Ipd, 7, 0, "div", 1);
				}
				if (a & 2) {
					_.Uh(b.XOb()), _.y(2), _.Bh(b.navItems), _.y(2), a = _.Uh(b.pinnedApplets()), _.y(), b = _.Uh(b.YLa()), _.y(), _.C(a.length > 0 || b.length > 0 ? 6 : -1), _.y(), _.C(a.length > 0 ? 7 : -1), _.y(), _.C(b.length > 0 ? 8 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.OD,
				_.ND,
				_.IC,
				_.HC,
				_.sA,
				_.D3,
				_.Cz,
				_.Bz
			],
			styles: ["ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:12px;padding-bottom:24px}li[_ngcontent-%COMP%]{margin-bottom:4px}.applets-list[_ngcontent-%COMP%]   li[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;margin-left:8px}.applets-list[_ngcontent-%COMP%]   li[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]{padding:2px 6px;border-radius:8px}.applets-list[_ngcontent-%COMP%]   li[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover);color:var(--color-v3-text)}.pin-button[_ngcontent-%COMP%], a[_ngcontent-%COMP%]{color:var(--color-v3-text-var)}.applet-link[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex-grow:1;-moz-box-flex:1;-ms-flex-positive:1;flex-grow:1;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.pinned-applets-header[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;margin-left:12px;margin-block:6px}.pin-button[_ngcontent-%COMP%]{-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;height:24px;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;visibility:hidden;width:24px}li[_ngcontent-%COMP%]:focus-within   .pin-button[_ngcontent-%COMP%], li[_ngcontent-%COMP%]:hover   .pin-button[_ngcontent-%COMP%]{visibility:visible}.pinned[_ngcontent-%COMP%]   .ms-button-icon-symbol[_ngcontent-%COMP%]{font-variation-settings:\"FILL\" 1,\"ROND\" 50,\"wght\" 400,\"GRAD\" 0,\"opsz\" 24}"]
		});
		var Jpd = [
			{
				text: "Start",
				routerLink: "apps",
				queryParams: { source: "" }
			},
			{
				text: "Gallery",
				routerLink: "apps",
				queryParams: { source: "showcase" },
				ve: 268858
			},
			{
				text: "Your apps",
				routerLink: "apps",
				queryParams: { source: "user" },
				ve: 268828
			}
		];
		var Ktd = function(a) {
			var b = a.F.url.includes("spend");
			if (a.U && !b) {
				var c = _.kd(_.qA(a.F, a.F.bk(["spend"])));
				b = _.H3(c);
				let d = _.G3(c);
				c = _.F3(c);
				let e = _.I3();
				if (!(_.N3(a.A, b) || _.N3(a.A, c) || _.N3(a.A, e))) {
					setTimeout(() => {
						a.A.Fi.next(d);
					}, 1e3);
				}
			}
		};
		var Ltd = class {
			constructor() {
				this.Qc = _.m(_.BM);
				this.H = _.m(c4);
				this.F = _.m(_.Cl);
				this.A = _.m(_.O3);
				this.R = _.m(_.Op);
				this.I = _.m(_.yG);
				this.U = this.R.getFlag(_.iL);
				this.S = _.Dk;
				this.Ye = _.E3;
				this.Zv = _.Wy;
				this.Yra = this.H.F;
				_.Fk([this.Qc.isNavbarExpanded, this.I.A], () => {
					if (this.Qc.isNavbarExpanded() && this.I.A()) {
						Ktd(this);
					} else {
						_.M3(this.A);
					}
				});
			}
			Eu(a) {
				X3(this.H, a.text);
			}
			GAa(a) {
				var b;
				return ((b = a.routerLink) == null ? 0 : b.includes(this.Zv.Nea)) ? "spend-nav-item" : "";
			}
		};
		Ltd.J = function(a) {
			return new (a || Ltd)();
		};
		Ltd.ka = _.u({
			type: Ltd,
			da: [["ms-nav-items-dashboard"]],
			ha: 3,
			ia: 0,
			la: [
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick",
					"xapTourElementId"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"isIconPositionEnd",
					"true",
					"target",
					"_blank",
					"rel",
					"noopener noreferrer",
					3,
					"iconName",
					"href",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"click",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick",
					"xapTourElementId"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "ul"), _.Ah(1, Npd, 3, 1, "li", null, Jtd), _.H();
				}
				if (a & 2) {
					_.y(), _.Bh(b.Yra);
				}
			},
			dependencies: [
				_.Yy,
				_.sA,
				_.D3,
				_.Cz,
				_.Bz,
				_.P3
			],
			styles: ["ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}[_nghost-%COMP%]{display:block}li[_ngcontent-%COMP%]{margin-bottom:4px}"]
		});
		var Ntd = class {
			constructor() {
				this.S = _.Dk;
				this.expanded = _.V(false);
				this.F = _.m(_.V3);
				this.Jf = _.m(_.UH);
				this.A = _.m(_.jH);
				this.jPa = _.m(_.rF);
				this.Qc = _.m(_.BM);
				this.R = _.m(_.ZC);
				this.hPa = _.m(_.O3);
				this.I = _.m(_.Cl);
				this.ve = {
					qmb: 298886,
					Lnb: 227421,
					Nnb: 227420
				};
				this.WTa = {
					text: "View all history",
					icon: "history",
					routerLink: "library"
				};
				this.Ye = _.E3;
				this.Rsa = 5;
				this.iPa = this.R.A.ey;
				this.Sa = this.F.Sa;
				this.Wd = _.Ck(this.jPa.Xm, { initialValue: undefined });
				this.U = this.F.UM;
				this.UM = _.W(() => this.U().slice(0, 5));
				this.H = _.W(() => {
					var b = this.Wd();
					return b && !this.Jf.Gx() || b && !this.Jf.U();
				});
				var a = true;
				_.Fk([this.H, this.A.F], () => {
					if (this.H() && (this.A.F() || a)) {
						a = false, _.Dod(this.F, this.I.url !== "/library"), this.A.A.set(false);
					}
				});
			}
		};
		Ntd.J = function(a) {
			return new (a || Ntd)();
		};
		Ntd.ka = _.u({
			type: Ntd,
			da: [["ms-prompt-history-v3"]],
			inputs: { expanded: [1, "expanded"] },
			ha: 2,
			ia: 3,
			la: [
				[
					"ms-button",
					"",
					1,
					"enable-drive-button",
					3,
					"ve",
					"veImpression",
					"veClick"
				],
				[3, "expanded"],
				[
					"ms-button",
					"",
					1,
					"enable-drive-button",
					3,
					"click",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					3,
					"afterExpand",
					"afterCollapse",
					"expanded"
				],
				[
					"ms-button",
					"",
					"size",
					"small",
					"variant",
					"borderless",
					"matTooltipPosition",
					"right",
					1,
					"prompt-link-wrapper",
					3,
					"routerLinkActive",
					"routerLinkActiveOptions",
					"matTooltip"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"data-test-view-all-history-link",
					"",
					1,
					"view-all-history-link",
					3,
					"iconName",
					"isIconPositionEnd",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					1,
					"prompt-link",
					3,
					"click",
					"routerLink",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"size",
					"small",
					3,
					"promptItem",
					"tooltipPosition"
				],
				[1, "info-message"],
				[1, "prompt-history-loading-shimmer"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"data-test-view-all-history-link",
					"",
					1,
					"view-all-history-link",
					3,
					"click",
					"iconName",
					"isIconPositionEnd",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Th(0), _.B(1, Xpd, 2, 1);
				}
				if (a & 2) {
					_.Uh(b.UM() || _.zi(2, Mtd)), _.y(), _.C(b.Wd() !== undefined ? 1 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.OE,
				_.KE,
				_.IC,
				_.HC,
				_.W3,
				_.sA,
				_.D3,
				_.Cz,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{-webkit-user-select:none;-moz-user-select:none;-ms-user-select:none;user-select:none}[_nghost-%COMP%]   mat-expansion-panel[_ngcontent-%COMP%]{--mat-expansion-container-background-color:transparent;--mat-expansion-container-elevation-shadow:none}.enable-drive-button[_ngcontent-%COMP%], .prompt-link[_ngcontent-%COMP%]{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.view-all-history-link[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;color:var(--color-v3-text-var)}.view-all-history-link[_ngcontent-%COMP%]:focus-visible, .view-all-history-link[_ngcontent-%COMP%]:hover{background:var(--color-nav-item-hover);color:var(--color-v3-text)}.view-all-history-link.active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text)}ul[_ngcontent-%COMP%]{overflow:hidden;margin-block:0;padding-left:0;-webkit-transition:max-height .2s ease-in,opacity .2s ease-in,padding-top .2s ease-in,padding-bottom .2s ease-in;transition:max-height .2s ease-in,opacity .2s ease-in,padding-top .2s ease-in,padding-bottom .2s ease-in}ul[_ngcontent-%COMP%]   li[_ngcontent-%COMP%]{margin-block:2px;padding-right:0}ul[_ngcontent-%COMP%]   li[_ngcontent-%COMP%]   .prompt-link[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;color:var(--color-v3-text-var);font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px}ul[_ngcontent-%COMP%]   li[_ngcontent-%COMP%]   ms-prompt-options-menu[_ngcontent-%COMP%]{display:none}@media screen and (max-width:600px){ul[_ngcontent-%COMP%]   li[_ngcontent-%COMP%]   ms-prompt-options-menu[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}}ul[_ngcontent-%COMP%]   li.active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text)}ul[_ngcontent-%COMP%]   li.prompt-link-wrapper[_ngcontent-%COMP%]:focus-within, ul[_ngcontent-%COMP%]   li.prompt-link-wrapper[_ngcontent-%COMP%]:hover{background:var(--color-nav-item-hover)}ul[_ngcontent-%COMP%]   li.prompt-link-wrapper[_ngcontent-%COMP%]:focus-within   .prompt-link[_ngcontent-%COMP%], ul[_ngcontent-%COMP%]   li.prompt-link-wrapper[_ngcontent-%COMP%]:hover   .prompt-link[_ngcontent-%COMP%]{color:var(--color-v3-text)}ul[_ngcontent-%COMP%]   li.prompt-link-wrapper[_ngcontent-%COMP%]:focus-within   ms-prompt-options-menu[_ngcontent-%COMP%], ul[_ngcontent-%COMP%]   li.prompt-link-wrapper[_ngcontent-%COMP%]:hover   ms-prompt-options-menu[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}.prompt-history-loading-shimmer[_ngcontent-%COMP%]{-webkit-animation-duration:1.7s;animation-duration:1.7s;-webkit-animation-fill-mode:forwards;animation-fill-mode:forwards;-webkit-animation-iteration-count:infinite;animation-iteration-count:infinite;-webkit-animation-timing-function:linear;animation-timing-function:linear;-webkit-animation-name:_ngcontent-%COMP%_moving-gradient;animation-name:_ngcontent-%COMP%_moving-gradient;background:-webkit-linear-gradient(160deg,var(--color-loading-background) 40%,var(--color-loading-background-contrast) 50%,var(--color-loading-background) 60%);background:linear-gradient(290deg,var(--color-loading-background) 40%,var(--color-loading-background-contrast) 50%,var(--color-loading-background) 60%);background-size:800px;border-radius:12px;padding-block:4px}@-webkit-keyframes _ngcontent-%COMP%_moving-gradient{0%{background-position:-400px 0}to{background-position:400px 0}}@keyframes _ngcontent-%COMP%_moving-gradient{0%{background-position:-400px 0}to{background-position:400px 0}}.enable-drive-button[_ngcontent-%COMP%], .info-message[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;margin-left:14px;margin-block:4px}"],
			data: { animation: std }
		});
		var Ptd = class {
			constructor() {
				this.S = _.Dk;
				this.H = _.m(_.rF);
				this.F = _.m(_.pG);
				this.window = _.m(_.Sn);
				this.A = _.m(c4);
				this.c9 = _.M(this.window.localStorage.getItem("is_prompt_history_expanded") === "false" ? false : true);
				this.Wd = _.Ck(this.H.Xm, { initialValue: undefined });
				this.Ye = _.E3;
				this.MI = Otd;
				this.ve = {
					I2: 281714,
					Psa: 281710
				};
				this.MCb = _.W(() => `${this.c9() ? "Collapse" : "Expand"} prompts history`);
				this.cDa = _.W(() => {
					var a = this.F.F;
					var b;
					var c;
					a = (c = (b = a().routeConfig) == null ? undefined : b.path) != null ? c : "";
					return a.startsWith("prompts") || a.startsWith("generate-speech") || a.startsWith("live");
				});
			}
			Eu(a) {
				X3(this.A, a.text);
			}
		};
		Ptd.J = function(a) {
			return new (a || Ptd)();
		};
		Ptd.ka = _.u({
			type: Ptd,
			da: [["ms-nav-items-playground"]],
			ha: 5,
			ia: 14,
			la: [
				[1, "wrapper"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					1,
					"playground-link",
					3,
					"click",
					"iconName",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"expandCollapseIconTooltipText()",
					"matTooltipPosition",
					"right",
					1,
					"history-button",
					3,
					"matTooltip",
					"expanded",
					"ve",
					"veImpression",
					"veClick"
				],
				[3, "expanded"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"expandCollapseIconTooltipText()",
					"matTooltipPosition",
					"right",
					1,
					"history-button",
					3,
					"click",
					"matTooltip",
					"ve",
					"veImpression",
					"veClick"
				],
				[3, "iconName"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 0)(1, "a", 1), _.J("click", function() {
						return b.Eu(b.MI);
					}), _.R(2), _.H(), _.B(3, Ypd, 2, 8, "button", 2), _.H(), _.I(4, "ms-prompt-history-v3", 3);
				}
				if (a & 2) {
					_.y(), _.P("has-home-nav", true)("active-override", b.cDa()), _.E("iconName", b.MI.icon)("routerLink", b.MI.routerLink)("routerLinkActive", b.Ye.class)("routerLinkActiveOptions", b.Ye.options)("ve", b.ve.Psa)("veImpression", true)("veClick", true), _.y(), _.S(" ", b.MI.text, " "), _.y(), _.C(b.Wd() ? 3 : -1), _.y(), _.E("expanded", b.c9());
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				_.IC,
				_.HC,
				Ntd,
				_.sA,
				_.D3,
				_.Cz,
				_.Bz
			],
			styles: ["ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:4px}.playground-link[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1}.history-button[_ngcontent-%COMP%]{margin-right:8px}.history-button[_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.history-button[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{-webkit-transition:-webkit-transform .2s ease;transition:-webkit-transform .2s ease;transition:transform .2s ease;transition:transform .2s ease,-webkit-transform .2s ease}.history-button.expanded[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{-webkit-transform:rotate(90deg);transform:rotate(90deg)}"],
			data: { animation: std }
		});
		var Qtd = function(a) {
			if (a.R) {
				var b = _.kd(_.qA(a.H, a.H.bk(["spend"])));
				let c = _.H3(b);
				let d = _.G3(b);
				b = _.F3(b);
				let e = _.I3();
				if (!(_.N3(a.A, d) || _.N3(a.A, b) || _.N3(a.A, e))) {
					setTimeout(() => {
						a.A.Fi.next(c);
					}, 1e3);
				}
			}
		};
		var Rtd = class {
			constructor() {
				this.Qc = _.m(_.BM);
				this.U = _.m(c4);
				this.H = _.m(_.Cl);
				this.A = _.m(_.O3);
				this.F = _.m(_.Op);
				this.X = _.m(_.pG);
				this.I = _.m(_.yG);
				this.gY = this.F.getFlag(_.Wzb);
				this.R = this.F.getFlag(_.iL);
				this.fya = "documentation";
				this.Jya = "https://ai.google.dev/gemini-api/docs";
				this.Ye = _.E3;
				this.S = _.Dk;
				this.Zra = "dashboard-nav-item";
				this.s2 = {
					text: "Build",
					icon: "design_services",
					routerLink: "apps"
				};
				this.eya = this.X.I;
				this.hG = {
					text: "Dashboard",
					icon: "speed"
				};
				this.ve = {
					tea: 281711,
					Nsa: 281712,
					H2: 281713
				};
				_.Fk([this.Qc.isNavbarExpanded, this.I.A], () => {
					if (this.Qc.isNavbarExpanded() && this.I.A()) {
						Qtd(this);
					} else {
						_.M3(this.A);
					}
				});
			}
			Eu(a) {
				X3(this.U, a.text, a.text !== this.hG.text);
			}
		};
		Rtd.J = function(a) {
			return new (a || Rtd)();
		};
		Rtd.ka = _.u({
			type: Rtd,
			da: [["ms-nav-items-main"]],
			ha: 11,
			ia: 18,
			la: [
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"click",
					"iconName",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick"
				],
				[1, "nav-item-main-text"],
				[3, "iconName"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"click",
					"xapTourElementId",
					"iconName",
					"routerLink",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"iconName",
					"routerLink",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"target",
					"_blank",
					3,
					"iconName",
					"href",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					1,
					"documentation-outward-icon",
					3,
					"iconName"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.I(0, "ms-nav-items-playground"), _.F(1, "a", 0), _.J("click", function() {
						return b.Eu(b.s2);
					}), _.F(2, "span", 1), _.R(3), _.H(), _.I(4, "span", 2), _.H(), _.F(5, "a", 3), _.J("click", function() {
						return b.Eu(b.hG);
					}), _.F(6, "span", 1), _.R(7), _.H(), _.I(8, "span", 2), _.H(), _.B(9, Zpd, 3, 5, "a", 4)(10, $pd, 4, 6, "a", 5);
				}
				if (a & 2) {
					_.y(), _.E("iconName", b.s2.icon)("routerLink", b.s2.routerLink)("routerLinkActive", b.Ye.class)("routerLinkActiveOptions", b.Ye.options)("ve", b.ve.tea)("veImpression", true)("veClick", true), _.y(2), _.S(" ", b.s2.text, " "), _.y(), _.E("iconName", b.S.gh), _.y(), _.E("xapTourElementId", b.Zra)("iconName", b.hG.icon)("routerLink", b.eya())("ve", b.ve.Nsa)("veImpression", true)("veClick", true), _.y(2), _.S(" ", b.hG.text, " "), _.y(), _.E("iconName", b.S.gh), _.y(), _.C(b.gY ? 9 : 10);
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				Ptd,
				_.sA,
				_.D3,
				_.Cz,
				_.Bz,
				_.P3
			],
			styles: ["ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.nav-item-main-text[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;text-align:left}a[_ngcontent-%COMP%]{margin-block:2px}"]
		});
		var Std = new Map([
			[0, {
				title: "",
				route: "prompts"
			}],
			[1, {
				title: "Dashboard",
				route: "apikey"
			}],
			[2, {
				title: "Build",
				route: "apps"
			}]
		]);
		var Ttd = class {
			constructor() {
				this.H = _.m(_.pG);
				this.F = _.m(c4);
				this.bK = ftd;
				this.S = _.Dk;
				this.ve = { Osa: 281715 };
				this.KKa = _.W(() => Std.get(this.ck()).title);
				this.A = _.W(() => {
					var a;
					var b;
					var c;
					return (c = (a = this.H.F()) == null ? undefined : (b = a.routeConfig) == null ? undefined : b.path) != null ? c : "";
				});
				this.ck = _.W(() => {
					var a = this.A();
					return Kpd(a) ? 2 : this.F.F.some((b) => b.routerLink === a) ? 1 : 0;
				});
				this.qEa = _.W(() => "/prompts/new_chat");
			}
		};
		Ttd.J = function(a) {
			return new (a || Ttd)();
		};
		Ttd.ka = _.u({
			type: Ttd,
			da: [["ms-nav-items"]],
			ha: 4,
			ia: 2,
			la: [[
				"ms-button",
				"",
				"variant",
				"borderless",
				1,
				"back-item",
				3,
				"iconName",
				"routerLink",
				"ve",
				"veClick",
				"veImpression"
			], [1, "nav_items_nav_mode"]],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, aqd, 2, 6, "a", 0), _.B(1, bqd, 1, 0, "ms-nav-items-main", 1)(2, cqd, 1, 0, "ms-nav-items-dashboard", 1)(3, dqd, 1, 0, "ms-nav-items-console", 1);
				}
				if (a & 2) {
					a = b.ck(), _.C(a !== b.bK.uea ? 0 : -1), _.y(), _.C(a === b.bK.uea ? 1 : a === b.bK.DASHBOARD ? 2 : a === b.bK.fG ? 3 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.IC,
				Itd,
				Ltd,
				Rtd,
				_.sA,
				_.Cz,
				_.Bz
			],
			styles: ["ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.back-item[_ngcontent-%COMP%]{margin-bottom:2px}.nav_items_nav_mode[_ngcontent-%COMP%]{opacity:1}@-webkit-keyframes _ngcontent-%COMP%_slideInFromLeft{0%{-webkit-transform:translateX(-100%);transform:translateX(-100%);opacity:0}to{-webkit-transform:translateX(0);transform:translateX(0);opacity:1}}@keyframes _ngcontent-%COMP%_slideInFromLeft{0%{-webkit-transform:translateX(-100%);transform:translateX(-100%);opacity:0}to{-webkit-transform:translateX(0);transform:translateX(0);opacity:1}}.slide-in-from-left[_ngcontent-%COMP%]{-webkit-animation:_ngcontent-%COMP%_slideInFromLeft .7s cubic-bezier(.23,1,.32,1);animation:_ngcontent-%COMP%_slideInFromLeft .7s cubic-bezier(.23,1,.32,1)}@-webkit-keyframes _ngcontent-%COMP%_slideInFromRight{0%{-webkit-transform:translateX(100%);transform:translateX(100%);opacity:0}to{-webkit-transform:translateX(0);transform:translateX(0);opacity:1}}@keyframes _ngcontent-%COMP%_slideInFromRight{0%{-webkit-transform:translateX(100%);transform:translateX(100%);opacity:0}to{-webkit-transform:translateX(0);transform:translateX(0);opacity:1}}.slide-in-from-right[_ngcontent-%COMP%]{-webkit-animation:_ngcontent-%COMP%_slideInFromRight .7s cubic-bezier(.23,1,.32,1);animation:_ngcontent-%COMP%_slideInFromRight .7s cubic-bezier(.23,1,.32,1)}"]
		});
		var Xtd = function(a) {
			if (a.fa) {
				let b = _.kd(_.qA(a.H, a.H.bk(["spend"])));
				let c = _.H3(b);
				let d = _.G3(b);
				if (!(_.N3(a.A, c) || _.N3(a.A, d))) {
					setTimeout(() => {
						if (!a.Qc.isNavbarExpanded()) {
							var e = a.A;
							var f = _.F3(b);
							e.Fi.next(f);
						}
					}, 1e3);
				}
			}
		};
		var Ytd = class {
			constructor() {
				this.F = _.m(_.Ou);
				this.ma = _.m(_.pG);
				this.X = _.m(_.Qu);
				this.Qc = _.m(_.BM);
				_.m(c4);
				this.D4 = _.m(_.s3);
				this.Vb = _.m(_.AG);
				this.na = _.m(_.OC);
				this.A = _.m(_.O3);
				this.H = _.m(_.Cl);
				this.U = _.m(_.Op);
				this.Ig = _.m(_.C3);
				this.I = _.m(_.yG);
				this.fa = this.U.getFlag(_.iL);
				this.Aea = "nav-button";
				this.V3 = _.Ni(Y3);
				this.n$ = _.Ni("msAccountSwitcher");
				this.Ge = _.Jp;
				this.S = _.Dk;
				this.rxa = `${_.Aa() ? "⌘" : "Ctrl"} /`;
				this.ve = { Ojb: 262630 };
				this.Oe = this.X.Oe;
				this.url = this.ma.url;
				this.vHa = Utd;
				this.R = [
					"prompts",
					"live",
					"generate-speech",
					"new_music"
				];
				this.PJa = _.W(() => {
					if (!this.url()) return true;
					var a = this.url().split("?")[0];
					return !(a === "/" || this.R.some((b) => a.startsWith(`/${b}`)));
				});
				this.nEa = _.W(() => {
					var a = this.url();
					return a && a.startsWith("/apps/") ? "/apps" : "/prompts/new_chat";
				});
				this.yf = this.Vb.yf;
				this.Bu = this.Vb.Bu;
				this.aa = this.Vb.tj;
				this.Hb = _.W(() => _.Nn(this.Oe));
				this.QJa = _.W(() => this.Ge() && this.aa() && !this.yf() && !this.Hb() && !this.bb() && this.ea());
				this.Le = this.Vb.Le;
				this.bb = this.na.bb;
				this.Uba = _.W(() => this.Le() && this.yf() || !!this.bb());
				this.V9 = _.W(() => {
					var a;
					var b;
					return ((a = this.bb()) == null ? undefined : (b = _.Io(a)) == null ? undefined : b.slice(-4)) || "";
				});
				this.o9 = _.W(() => _.qp(this.url()));
				this.ea = _.W(() => this.o9() || this.url().startsWith("/apps"));
				_.Fk([this.Qc.isNavbarExpanded, this.I.A], () => {
					if (!this.Qc.isNavbarExpanded() && this.I.A()) {
						Xtd(this);
					} else {
						_.M3(this.A);
					}
				});
			}
			iu() {
				_.Rn(this.F, "NAV", "Toggled Nav");
				this.Qc.iu();
			}
			kxa() {
				_.Rn(this.F, "NAV", "Clicked Logo");
			}
			jxa(a) {
				a.stopPropagation();
				if (this.Ge()) {
					let b;
					if (!((b = this.n$()) == null)) {
						b.nO();
					}
				} else {
					let b;
					if (!((b = this.V3()) == null)) {
						b.nO();
					}
				}
			}
			HGa() {
				this.Ig.open(2);
			}
		};
		Ytd.J = function(a) {
			return new (a || Ytd)();
		};
		Ytd.ka = _.u({
			type: Ytd,
			da: [["ms-navbar"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.V3, Y3, 5)(b.n$, Vtd, 5);
				}
				if (a & 2) {
					_.ki(2);
				}
			},
			ha: 31,
			ia: 15,
			la: [
				["productsMenu", "matMenu"],
				["accountSwitcher", ""],
				["msAccountSwitcher", ""],
				[
					"ms-button",
					"",
					"variant",
					"icon-primary",
					"matTooltip",
					"Toggle navigation menu",
					"aria-label",
					"Toggle navigation menu",
					1,
					"floating-toggle-button",
					3,
					"xapTourElementId",
					"expanded",
					"banner-visible",
					"iconName"
				],
				[
					1,
					"nav-content",
					"v3-left-nav",
					"nesting-enabled"
				],
				[
					1,
					"navbar-header",
					"v3-design"
				],
				[1, "logo-wrapper"],
				[
					1,
					"logo-link",
					3,
					"click",
					"routerLink"
				],
				["type", "lockup"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"View related products",
					1,
					"dropdown-trigger",
					3,
					"iconName",
					"matMenuTriggerFor"
				],
				[1, "products-menu"],
				[1, "products-menu-header"],
				[
					"mat-menu-item",
					"",
					"target",
					"_blank",
					"rel",
					"noopener noreferrer",
					1,
					"product-menu-item",
					3,
					"href"
				],
				[1, "empty-space"],
				[1, "bottom-actions"],
				[3, "isNavbarExpanded"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"matTooltipPosition",
					"right",
					1,
					"command-palette-button",
					3,
					"click",
					"iconName"
				],
				[1, "label"],
				[1, "shortcut"],
				[3, "ngTemplateOutlet"],
				[
					"ms-button",
					"",
					"variant",
					"icon-primary",
					"matTooltip",
					"Toggle navigation menu",
					"aria-label",
					"Toggle navigation menu",
					1,
					"floating-toggle-button",
					3,
					"click",
					"xapTourElementId",
					"iconName"
				],
				[1, "product-icon-container"],
				[
					1,
					"product-icon",
					3,
					"iconName"
				],
				[1, "product-content"],
				[1, "product-name"],
				[1, "product-description"],
				[
					1,
					"external-link-icon",
					3,
					"iconName"
				],
				[1, "account-switcher-container"],
				[
					"ms-button",
					"",
					1,
					"account-switcher-button",
					3,
					"click",
					"variant"
				],
				[1, "avatar-placeholder"],
				[1, "account-switcher-text"],
				[
					1,
					"navbar-badge",
					"hide-circle"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Th(0), _.B(1, eqd, 1, 7, "button", 3), _.F(2, "div", 4)(3, "div", 5)(4, "div", 6)(5, "a", 7), _.J("click", function() {
						return b.kxa();
					}), _.I(6, "ms-logo-icon", 8), _.H(), _.I(7, "button", 9), _.H()(), _.F(8, "mat-menu", 10, 0)(10, "div", 11), _.R(11, "Products and apps"), _.H(), _.Ah(12, fqd, 9, 5, "a", 12, Wtd), _.H(), _.F(14, "nav"), _.I(15, "ms-nav-items")(16, "div", 13), _.F(17, "div", 14), _.I(18, "ms-navbar-disclaimer", 15), _.B(19, iqd, 3, 0), _.F(20, "button", 16), _.J("click", function() {
						return b.HGa();
					}), _.F(21, "span", 17), _.R(22, "Search"), _.H(), _.F(23, "span", 18), _.R(24), _.H()(), _.I(25, "ms-updates")(26, "ms-api-key-button")(27, "ms-settings-menu"), _.Ih(28, 19), _.H()()(), _.z(29, tqd, 6, 7, "ng-template", null, 1, _.Ii);
				}
				if (a & 2) {
					a = _.O(9);
					let c = _.O(30);
					let d = _.Uh(b.Qc.isNavbarExpanded());
					_.y();
					_.C(b.PJa() ? 1 : -1);
					_.y();
					_.P("expanded", d)("collapsed", !d);
					_.E("@parent", undefined);
					_.y(3);
					_.E("routerLink", b.nEa());
					_.y(2);
					_.E("iconName", b.S.Ck)("matMenuTriggerFor", a);
					_.y(5);
					_.Bh(b.vHa);
					_.y(6);
					_.E("isNavbarExpanded", d);
					_.y();
					_.C(b.QJa() ? 19 : -1);
					_.y();
					_.E("iconName", b.S.Lm);
					_.y(4);
					_.U(b.rxa);
					_.y(4);
					_.E("ngTemplateOutlet", c);
				}
			},
			dependencies: [
				Y3,
				Z3,
				_.Yy,
				_.tz,
				_.nz,
				_.dz,
				_.tG,
				_.OD,
				_.OE,
				_.$D,
				_.yA,
				_.fF,
				_.wI,
				_.tI,
				_.sI,
				_.vI,
				_.IC,
				_.HC,
				Ttd,
				b4,
				_.sA,
				$3,
				a4,
				_.Cz,
				_.P3
			],
			styles: [".nav-content[_ngcontent-%COMP%]{-webkit-user-select:none;-moz-user-select:none;-ms-user-select:none;user-select:none;background-color:var(--color-v3-surface-left-nav);border-right:1px solid var(--color-v3-surface-left-nav-border);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;height:100%;overflow:hidden auto;padding:0 18px;scrollbar-gutter:unset;-webkit-transition:width .2s cubic-bezier(.4,0,.2,0),opacity .2s cubic-bezier(.4,0,.2,0),left .2s cubic-bezier(.4,0,.2,0),padding .2s cubic-bezier(.4,0,.2,0);transition:width .2s cubic-bezier(.4,0,.2,0),opacity .2s cubic-bezier(.4,0,.2,0),left .2s cubic-bezier(.4,0,.2,0),padding .2s cubic-bezier(.4,0,.2,0);z-index:5}.nav-content.collapsed[_ngcontent-%COMP%]{width:80px}.nav-content.collapsed.v3-left-nav[_ngcontent-%COMP%]{width:0;padding:0}.nav-content.expanded[_ngcontent-%COMP%]{width:220px}.nav-content.nesting-enabled[_ngcontent-%COMP%]{padding-left:8px;padding-right:8px}@media screen and (max-width:960px){.nav-content[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container);bottom:0;left:0;position:fixed;top:0;-webkit-transform:translate(-100%);transform:translate(-100%);-webkit-transition:opacity .2s ease-out,-webkit-transform .2s ease-out;transition:opacity .2s ease-out,-webkit-transform .2s ease-out;transition:transform .2s ease-out,opacity .2s ease-out;transition:transform .2s ease-out,opacity .2s ease-out,-webkit-transform .2s ease-out}.nav-content.expanded[_ngcontent-%COMP%]{opacity:1;-webkit-transform:translate(0);transform:translate(0);-webkit-transition:opacity .2s ease-in,-webkit-transform .2s ease-in;transition:opacity .2s ease-in,-webkit-transform .2s ease-in;transition:transform .2s ease-in,opacity .2s ease-in;transition:transform .2s ease-in,opacity .2s ease-in,-webkit-transform .2s ease-in}}.floating-toggle-button[_ngcontent-%COMP%]{position:absolute;top:16px;left:20px;-webkit-transition:translate .2s cubic-bezier(.4,0,.2,0);transition:translate .2s cubic-bezier(.4,0,.2,0);z-index:3}.floating-toggle-button.banner-visible[_ngcontent-%COMP%]{top:76px}@media screen and (max-width:600px){.floating-toggle-button[_ngcontent-%COMP%]{translate:-8px}}@media screen and (min-width:769px){.floating-toggle-button.expanded[_ngcontent-%COMP%]{translate:220px 0}}.navbar-header[_ngcontent-%COMP%]{position:-webkit-sticky;position:sticky;top:0;z-index:2;background:inherit}.navbar-header.v3-design[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;padding:16px 0;width:100%}.navbar-header[_ngcontent-%COMP%]   .logo-wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;width:100%;gap:4px;padding-left:12px}.navbar-header[_ngcontent-%COMP%]   .dropdown-trigger[_ngcontent-%COMP%]{color:var(--color-v3-text-var);margin-right:8px}.navbar-header[_ngcontent-%COMP%]   .logo-link[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;text-decoration:none;color:inherit;max-width:140px}nav[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:1;-webkit-flex:1 0;-moz-box-flex:1;-ms-flex:1 0;flex:1 0;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}.computer-use-case-nav-item[_ngcontent-%COMP%]{padding:10px 8px;margin-block:2px}.computer-use-case-nav-item[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]{cursor:pointer}.computer-use-case-nav-item[_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.empty-space[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1}.empty-space.clickable[_ngcontent-%COMP%]{margin-inline:-18px;cursor:pointer}.bottom-actions[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;padding-bottom:16px}.bottom-actions[_ngcontent-%COMP%] > [ms-button][_ngcontent-%COMP%]{color:var(--color-v3-text-var)}.bottom-actions[_ngcontent-%COMP%] > [ms-button][_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.bottom-actions[_ngcontent-%COMP%]   ms-navbar-disclaimer[_ngcontent-%COMP%]{margin-bottom:4px}.bottom-actions[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%], .bottom-actions[_ngcontent-%COMP%]   .command-palette-button[_ngcontent-%COMP%]{-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;width:100%}.bottom-actions[_ngcontent-%COMP%]   .command-palette-button[_ngcontent-%COMP%]   .shortcut[_ngcontent-%COMP%]{color:var(--color-v3-text-var);margin-left:auto;padding-left:4px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;opacity:0;-webkit-transition:opacity .15s ease;transition:opacity .15s ease}.bottom-actions[_ngcontent-%COMP%]   .command-palette-button[_ngcontent-%COMP%]:hover   .shortcut[_ngcontent-%COMP%]{opacity:1}.account-switcher-container[_ngcontent-%COMP%]{position:relative}.account-switcher-container[_ngcontent-%COMP%]:has(.v3-container-485387979){height:48px}.account-switcher-container[_ngcontent-%COMP%]   alkali-accountswitcher[_ngcontent-%COMP%]{position:absolute;left:13px;top:9px}.account-switcher-container[_ngcontent-%COMP%]   ms-account-switcher[_ngcontent-%COMP%]{position:absolute;left:13px;top:16px}.account-switcher-container[_ngcontent-%COMP%]   .avatar-placeholder[_ngcontent-%COMP%]{width:18px;-webkit-box-flex:0;-webkit-flex:0 0 18px;-moz-box-flex:0;-ms-flex:0 0 18px;flex:0 0 18px}.account-switcher-container[_ngcontent-%COMP%]   .avatar-placeholder.g1-member[_ngcontent-%COMP%]{width:24px;-webkit-box-flex:0;-webkit-flex:0 0 24px;-moz-box-flex:0;-ms-flex:0 0 24px;flex:0 0 24px}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%]{width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;color:var(--color-v3-text-var)}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button.v3-container-485387979[_ngcontent-%COMP%]{border:1px solid var(--color-v3-outline-var);background-color:var(--color-v3-surface-container);height:auto;margin-top:4px;padding:8px 12px}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button.v3-container-485387979[_ngcontent-%COMP%]:hover{background-color:var(--color-v3-hover)}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button.v3-container-485387979[_ngcontent-%COMP%]   .account-switcher-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var);line-height:26px}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%]   .account-switcher-text[_ngcontent-%COMP%]{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%]   .navbar-badge[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;background-color:var(--color-v3-button-container);border-radius:8px;padding:4px 8px;margin-left:4px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;text-transform:uppercase}  .products-menu.mat-mdc-menu-panel{border-radius:12px;border:none;box-shadow:var(--v3-shadow-sm);background-color:var(--color-v3-surface-container);min-width:320px;overflow:visible}  .products-menu .mat-mdc-menu-content{padding:4px;background:none}  .products-menu .products-menu-header{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;font-weight:500;color:var(--color-v3-text-var);padding:8px 8px 4px;margin-left:0}  .products-menu.mat-mdc-menu-panel div.mat-mdc-menu-content a.product-menu-item.mat-mdc-menu-item{border-radius:12px}  .products-menu.mat-mdc-menu-panel div.mat-mdc-menu-content a.product-menu-item.mat-mdc-menu-item .product-icon.material-symbols-outlined{margin:0}  .products-menu .product-menu-item{border-radius:12px;padding:8px;-webkit-transition:background-color .15s cubic-bezier(.4,0,.2,1),color .15s cubic-bezier(.4,0,.2,1);transition:background-color .15s cubic-bezier(.4,0,.2,1),color .15s cubic-bezier(.4,0,.2,1)}  .products-menu .product-menu-item:hover{background-color:var(--color-v3-hover)}  .products-menu .product-menu-item .mat-mdc-menu-item-text{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;width:100%}  .products-menu .product-menu-item .product-icon-container{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:32px;height:32px;border-radius:12px;background-color:var(--color-v3-hover);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}  .products-menu .product-menu-item .product-icon{font-size:16px;line-height:1;color:var(--color-v3-text-var)}  .products-menu .product-menu-item .product-content{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:2px;-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;min-width:0}  .products-menu .product-menu-item .product-name{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text)}  .products-menu .product-menu-item .product-description{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}  .products-menu .product-menu-item .external-link-icon{font-size:16px;color:var(--color-v3-text-var);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;opacity:0;-webkit-transition:opacity .15s ease;transition:opacity .15s ease}  .products-menu .product-menu-item:hover .external-link-icon{opacity:1}"],
			data: { animation: std }
		});
		var aud = function(a) {
			var b = a.F.url.includes("spend");
			if (a.U && !b) {
				var c = _.kd(_.qA(a.F, a.F.bk(["spend"])));
				b = _.H3(c);
				let d = _.G3(c);
				c = _.F3(c);
				let e = _.I3();
				if (!(_.N3(a.A, d) || _.N3(a.A, b) || _.N3(a.A, c) || _.N3(a.A, e))) {
					setTimeout(() => {
						a.A.Fi.next(d);
					}, 1e3);
				}
			}
		};
		var bud = class {
			constructor() {
				this.Qc = _.m(_.BM);
				this.H = _.m(c4);
				this.F = _.m(_.Cl);
				this.A = _.m(_.O3);
				this.R = _.m(_.Op);
				this.I = _.m(_.yG);
				this.U = this.R.getFlag(_.iL);
				this.S = _.Dk;
				this.Ye = Ztd;
				this.Zv = _.Wy;
				this.Yra = this.H.F;
				_.Fk([this.Qc.isNavbarExpanded, this.I.A], () => {
					if (this.Qc.isNavbarExpanded() && this.I.A()) {
						aud(this);
					} else {
						_.M3(this.A);
					}
				});
			}
			Eu(a) {
				X3(this.H, a.text);
			}
			GAa(a) {
				var b;
				return ((b = a.routerLink) == null ? 0 : b.includes(this.Zv.Nea)) ? "spend-nav-item" : "";
			}
		};
		bud.J = function(a) {
			return new (a || bud)();
		};
		bud.ka = _.u({
			type: bud,
			da: [["ms-nav-items-dashboard-v2"]],
			ha: 3,
			ia: 0,
			la: [
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick",
					"xapTourElementId"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"isIconPositionEnd",
					"true",
					"target",
					"_blank",
					"rel",
					"noopener noreferrer",
					3,
					"iconName",
					"href",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"click",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick",
					"xapTourElementId"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "ul"), _.Ah(1, wqd, 3, 1, "li", null, $td), _.H();
				}
				if (a & 2) {
					_.y(), _.Bh(b.Yra);
				}
			},
			dependencies: [
				_.Yy,
				_.sA,
				_.D3,
				_.Cz,
				_.Bz,
				_.P3
			],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{padding-left:8px;padding-right:8px;border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]   .ms-button-icon-symbol[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:24px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}[_nghost-%COMP%]{display:block}li[_ngcontent-%COMP%]{margin-bottom:4px}"]
		});
		var d4 = class {
			constructor() {
				this.Ga = _.m(_.Jf);
				this.YG = _.Oi();
				this.Tc = _.Li.required();
				this.Fqa = _.Li.required();
				this.w7a = _.Li.required();
				this.hXa = _.Li.required();
				this.F = _.m(c4);
				this.close = _.Ki();
				this.Z6a = _.Ki();
				this.lN = [{
					Lb: "end",
					Mb: "top",
					Bb: "start",
					Gb: "top",
					offsetX: 8
				}, {
					Lb: "end",
					Mb: "bottom",
					Bb: "start",
					Gb: "bottom",
					offsetX: 8
				}];
				_.Fk([this.F.A], () => {
					if (this.F.A() !== this.w7a() && this.Tc()) {
						this.close.emit();
					}
				});
				_.Fk([this.Tc], (a) => {
					if (this.Tc()) {
						let b = (c) => {
							c = c.target;
							var d;
							var e;
							if (!(this.hXa().contains(c) || this.Ga.nativeElement.contains(c) || ((d = c.closest) == null ? 0 : d.call(c, ".cdk-overlay-container"))) || ((e = c.closest) == null ? 0 : e.call(c, "a"))) {
								this.close.emit();
							}
						};
						document.addEventListener("click", b);
						a(() => {
							document.removeEventListener("click", b);
							this.Nr();
						});
					}
				});
				_.Fk([this.Fqa], (a) => {
					var b;
					var c;
					var d = (b = this.Fqa()) == null ? undefined : (c = b.Ga) == null ? undefined : c.nativeElement;
					if (d) {
						let e = () => this.Nr();
						let f = () => this.QU();
						d.addEventListener("mouseenter", e);
						d.addEventListener("mouseleave", f);
						a(() => {
							d.removeEventListener("mouseenter", e);
							d.removeEventListener("mouseleave", f);
						});
					}
				});
			}
			QU() {
				this.Nr();
				this.A = setTimeout(() => {
					this.close.emit();
				}, 300);
			}
			Nr() {
				if (this.A) {
					clearTimeout(this.A), this.A = undefined;
				}
			}
		};
		d4.J = function(a) {
			return new (a || d4)();
		};
		d4.ka = _.u({
			type: d4,
			da: [["ms-nav-popover"]],
			Ud: function(a, b, c) {
				if (a & 1) {
					_.ii(c, b.YG, _.Zh, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			inputs: {
				Tc: [1, "isOpen"],
				Fqa: [1, "triggerOrigin"],
				w7a: [1, "popoverId"],
				hXa: [1, "containerElement"]
			},
			outputs: {
				close: "close",
				Z6a: "panelKeydown"
			},
			ha: 1,
			ia: 5,
			la: [
				[
					"cdkConnectedOverlay",
					"",
					3,
					"cdkConnectedOverlayOrigin",
					"cdkConnectedOverlayOpen",
					"cdkConnectedOverlayPositions",
					"cdkConnectedOverlayHasBackdrop",
					"cdkConnectedOverlayPush"
				],
				[
					"cdkTrapFocus",
					"",
					1,
					"nav-flyout-panel",
					3,
					"mouseenter",
					"mouseleave",
					"keydown"
				],
				[1, "flyout-scroll-area"],
				[3, "ngTemplateOutlet"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.z(0, yqd, 3, 2, "ng-template", 0);
				}
				if (a & 2) {
					_.E("cdkConnectedOverlayOrigin", b.Fqa())("cdkConnectedOverlayOpen", b.Tc())("cdkConnectedOverlayPositions", b.lN)("cdkConnectedOverlayHasBackdrop", false)("cdkConnectedOverlayPush", true);
				}
			},
			dependencies: [
				_.GB,
				_.JA,
				_.HB,
				_.nz
			],
			styles: [".nav-flyout-panel[_ngcontent-%COMP%]{background:light-dark(hsla(0,0%,100%,.92),rgba(30,30,30,.92));-webkit-backdrop-filter:blur(12px);backdrop-filter:blur(12px);border-radius:16px;box-shadow:0 0 0 1px light-dark(rgba(0,0,0,.06),hsla(0,0%,100%,.08)),0 1px 1px -.4px rgba(0,0,0,.06),0 3px 3px 0 rgba(0,0,0,.06),0 6px 6px 0 rgba(0,0,0,.06),0 12px 12px 0 rgba(0,0,0,.04),0 24px 24px 0 rgba(0,0,0,.04);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;width:240px;max-height:400px;padding:6px}.flyout-scroll-area[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;overflow-y:auto;min-height:0}.flyout-scroll-area[_ngcontent-%COMP%]::-webkit-scrollbar{width:4px}.flyout-scroll-area[_ngcontent-%COMP%]::-webkit-scrollbar-thumb{background:light-dark(rgba(0,0,0,.12),hsla(0,0%,100%,.15));border-radius:2px}"],
			data: { animation: [_.qm("popover", [_.um(":enter", [_.sm({
				opacity: 0,
				transform: "translateX(-8px)"
			}), _.rm("200ms cubic-bezier(0.2, 0, 0, 1)", _.sm({
				opacity: 1,
				transform: "translateX(0)"
			}))]), _.um(":leave", [_.rm("150ms cubic-bezier(0.4, 0, 1, 1)", _.sm({
				opacity: 0,
				transform: "translateX(-8px)"
			}))])])] }
		});
		var iud = class {
			constructor() {
				this.S = _.Dk;
				this.applets = _.m(_.nI);
				this.A = _.m(c4);
				this.R = _.m(_.rF);
				this.U = _.m(_.ZC);
				this.Kh = _.lp();
				this.I = this.U.A.small;
				this.Id = _.W(() => this.Kh || this.I());
				this.Ga = _.m(_.Jf);
				this.xUa = _.Ni("appsTriggerLink");
				this.WDb = _.Ni("galleryTriggerLink");
				this.nU = _.hi();
				this.vc = _.M(localStorage.getItem("is_build_expanded") !== "false");
				this.NB = _.M(false);
				this.Wd = _.Ck(this.R.Xm, { initialValue: undefined });
				this.Ye = Ztd;
				this.Hga = hud;
				this.ve = {
					I2: 281714,
					tea: 281711,
					mhb: 268858,
					Fra: 268828
				};
				this.ky = "Untitled";
				this.H = _.W(() => this.applets.Cm.xc() ? this.applets.Cm.value() : []);
				this.pinnedApplets = _.W(() => this.H().filter((a) => _.Pm(a, 6)).slice(0, 5));
				this.YLa = _.W(() => this.H().filter((a) => !_.Pm(a, 6)).slice(0, 5));
				this.Sa = _.W(() => this.applets.Cm.Sa());
				this.Imb = 5;
				this.applets.ta.set(true);
				this.applets.Cm.reload();
			}
			bo(a) {
				a.preventDefault();
				a.stopPropagation();
				this.vc.update((b) => !b);
				localStorage.setItem("is_build_expanded", String(this.vc()));
				if (!this.vc()) {
					this.close();
				}
			}
			togglePopover() {
				if (this.NB()) {
					this.close();
				} else {
					this.JI();
				}
			}
			JI() {
				this.Nr();
				this.A.A.set("build-apps");
				this.NB.set(true);
			}
			close() {
				this.NB.set(false);
				var a = this.A;
				if (a.A() === "build-apps") {
					a.A.set(null);
				}
			}
			QU() {
				this.Nr();
				this.F = setTimeout(() => {
					this.close();
				}, 300);
			}
			Nr() {
				if (this.F) {
					clearTimeout(this.F), this.F = undefined;
				}
			}
			y_() {
				X3(this.A, "sub-item");
			}
			Eu(a) {
				X3(this.A, a.text);
			}
			v7(a) {
				return _.r3(a.Tf());
			}
			w7(a) {
				return _.wmd(a.Tf());
			}
			Bja(a) {
				return _.Xo(a.Tf());
			}
			QF(a, b) {
				_.xmd(this.applets, a);
				b.stopPropagation();
			}
			vGa(a) {
				if (a.key === "ArrowRight" || a.key === "ArrowDown") {
					this.JI(), setTimeout(() => {
						var b = this.nU().map((c) => c.nativeElement);
						if (b.length > 0) {
							b[0].focus();
						}
					}, 0), a.preventDefault();
				} else {
					if (a.key === "Tab") {
						this.close();
					}
				}
			}
			fGa(a) {
				var b = this.nU().map((d) => d.nativeElement);
				if (b.length !== 0) {
					var c = b.indexOf(document.activeElement);
					if (a.key === "ArrowDown") {
						let d;
						if (!((d = b[(c + 1) % b.length]) == null)) {
							d.focus();
						}
						a.preventDefault();
					} else if (a.key === "ArrowUp") {
						let d;
						if (!((d = b[(c - 1 + b.length) % b.length]) == null)) {
							d.focus();
						}
						a.preventDefault();
					} else if (a.key === "Escape" || a.key === "ArrowLeft") {
						this.close();
						let d;
						if (!((d = this.xUa()) == null)) {
							d.nativeElement.focus();
						}
						a.preventDefault();
					}
				}
			}
		};
		iud.J = function(a) {
			return new (a || iud)();
		};
		iud.ka = _.u({
			type: iud,
			da: [["ms-nav-items-build-v2"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.xUa, cud, 5)(b.WDb, dud, 5)(b.nU, eud, 5);
				}
				if (a & 2) {
					_.ki(3);
				}
			},
			ha: 7,
			ia: 22,
			la: [
				["galleryLink", ""],
				[
					"appsTriggerLink",
					"",
					"appsTrigger",
					"cdkOverlayOrigin"
				],
				["selectableItem", ""],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					1,
					"build-link",
					3,
					"click",
					"iconName",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick"
				],
				[1, "nav-item-main-text"],
				[
					"role",
					"button",
					"tabindex",
					"0",
					1,
					"expand-chevron",
					3,
					"click",
					"keydown.enter",
					"keydown.space",
					"iconName",
					"ve",
					"veImpression",
					"veClick"
				],
				[1, "sub-items"],
				[
					1,
					"sub-item",
					3,
					"routerLink",
					"queryParams",
					"routerLinkActive",
					"routerLinkActiveOptions"
				],
				[
					1,
					"sub-item",
					3,
					"click",
					"routerLink",
					"queryParams",
					"routerLinkActive",
					"routerLinkActiveOptions"
				],
				[
					"cdkOverlayOrigin",
					"",
					1,
					"sub-item",
					3,
					"click",
					"mouseenter",
					"focus",
					"keydown",
					"routerLink",
					"queryParams",
					"routerLinkActive",
					"routerLinkActiveOptions"
				],
				[
					"popoverId",
					"build-apps",
					3,
					"close",
					"panelKeydown",
					"containerElement",
					"isOpen",
					"triggerOrigin"
				],
				[1, "info-message"],
				[1, "flyout-section-label"],
				[
					1,
					"flyout-list",
					"pinned-list"
				],
				[1, "flyout-item"],
				[
					1,
					"flyout-item-link",
					3,
					"routerLink",
					"queryParams"
				],
				[
					"matTooltip",
					"Unpin",
					"matTooltipPosition",
					"right",
					"aria-label",
					"Unpin app",
					1,
					"flyout-pin-button",
					3,
					"click"
				],
				[3, "iconName"],
				[1, "flyout-list"],
				[
					"matTooltip",
					"Pin",
					"matTooltipPosition",
					"right",
					"aria-label",
					"Pin app",
					1,
					"flyout-pin-button",
					3,
					"click"
				],
				[
					1,
					"flyout-item",
					"applet-loading-shimmer"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Th(0)(1), _.F(2, "a", 3), _.J("click", function() {
						return b.Eu(b.Hga);
					}), _.F(3, "span", 4), _.R(4), _.H(), _.F(5, "span", 5), _.J("click", function(c) {
						return b.bo(c);
					})("keydown.enter", function(c) {
						return b.bo(c);
					})("keydown.space", function(c) {
						return b.bo(c);
					}), _.H()(), _.B(6, Qqd, 6, 8, "div", 6);
				}
				if (a & 2) {
					_.Uh(b.YLa() || _.zi(19, fud)), _.y(), _.Uh(b.pinnedApplets() || _.zi(20, fud)), _.y(), _.E("iconName", b.Hga.icon)("routerLink", b.Hga.routerLink)("routerLinkActive", b.Ye.class)("routerLinkActiveOptions", _.zi(21, gud))("ve", b.ve.tea)("veImpression", true)("veClick", true), _.y(2), _.U(b.Hga.text), _.y(), _.P("expanded", b.vc()), _.E("iconName", b.S.gh)("ve", b.ve.I2)("veImpression", true)("veClick", true), _.wh("aria-label", (b.vc() ? "Collapse" : "Expand") + " build apps")("aria-expanded", b.vc()), _.y(), _.C(b.vc() ? 6 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				_.IC,
				_.HC,
				d4,
				_.HB,
				_.FB,
				_.sA,
				_.D3,
				_.Cz,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{padding-left:8px;padding-right:8px;border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]   .ms-button-icon-symbol[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:24px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.nav-item-main-text[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;text-align:left}.expand-chevron[_ngcontent-%COMP%]{-webkit-transition:-webkit-transform .2s ease;transition:-webkit-transform .2s ease;transition:transform .2s ease;transition:transform .2s ease,-webkit-transform .2s ease;cursor:pointer}.expand-chevron.expanded[_ngcontent-%COMP%]{-webkit-transform:rotate(90deg);transform:rotate(90deg)}.expand-chevron[_ngcontent-%COMP%]:focus-visible{outline:auto}.sub-items[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:2px;padding-top:2px;margin-left:20px;padding-left:12px;border-left:1px solid light-dark(rgba(0,0,0,.12),hsla(0,0%,100%,.12))}.sub-item[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;height:32px;padding:0 12px 0 8px;border-radius:12px;text-decoration:none;color:var(--color-v3-text-var);cursor:pointer;-webkit-transition:background-color .15s ease-in-out;transition:background-color .15s ease-in-out}.sub-item[_ngcontent-%COMP%]:hover{background:var(--color-nav-item-hover);color:var(--color-v3-text)}.sub-item.active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.flyout-section-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:light-dark(rgba(0,0,0,.4),hsla(0,0%,100%,.4));padding:6px 8px 2px;text-transform:uppercase;letter-spacing:.4px}.flyout-divider[_ngcontent-%COMP%]{height:1px;margin:4px 8px;background:light-dark(rgba(0,0,0,.08),hsla(0,0%,100%,.1))}.flyout-list[_ngcontent-%COMP%]{list-style:none;margin:0;padding:0;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:1px}.flyout-item[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;height:32px;padding:0 8px;border-radius:10px;cursor:pointer;gap:8px;overflow:hidden;-webkit-transition:background-color .12s ease;transition:background-color .12s ease}.flyout-item[_ngcontent-%COMP%]:focus-within, .flyout-item[_ngcontent-%COMP%]:hover{background:light-dark(rgba(0,0,0,.05),hsla(0,0%,100%,.08))}.flyout-item[_ngcontent-%COMP%]:focus-within   .flyout-item-link[_ngcontent-%COMP%], .flyout-item[_ngcontent-%COMP%]:hover   .flyout-item-link[_ngcontent-%COMP%]{color:var(--color-v3-text)}.flyout-item[_ngcontent-%COMP%]:focus-within   .flyout-pin-button[_ngcontent-%COMP%], .flyout-item[_ngcontent-%COMP%]:hover   .flyout-pin-button[_ngcontent-%COMP%]{opacity:1}.flyout-item-link[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:light-dark(#5b5b64,hsla(0,0%,100%,.65));text-decoration:none;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.flyout-item-link[_ngcontent-%COMP%]:focus-visible{outline:auto;outline-offset:2px}.flyout-pin-button[_ngcontent-%COMP%]{all:unset;cursor:pointer;opacity:0;-webkit-transition:opacity .12s ease;transition:opacity .12s ease;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center}.flyout-pin-button[_ngcontent-%COMP%]:focus-visible{outline:auto;outline-offset:2px}.flyout-pin-button[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{font-size:16px;color:light-dark(#5b5b64,hsla(0,0%,100%,.65))}.pinned-list[_ngcontent-%COMP%]   .flyout-pin-button[_ngcontent-%COMP%]{opacity:1}.pinned-list[_ngcontent-%COMP%]   .flyout-pin-button[_ngcontent-%COMP%]   .material-symbols-outlined[_ngcontent-%COMP%]{font-variation-settings:\"FILL\" 1}.info-message[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;padding:8px;color:light-dark(#5b5b64,hsla(0,0%,100%,.65));margin:0}.applet-loading-shimmer[_ngcontent-%COMP%]{-webkit-animation-duration:1.7s;animation-duration:1.7s;-webkit-animation-fill-mode:forwards;animation-fill-mode:forwards;-webkit-animation-iteration-count:infinite;animation-iteration-count:infinite;-webkit-animation-timing-function:linear;animation-timing-function:linear;-webkit-animation-name:_ngcontent-%COMP%_moving-gradient;animation-name:_ngcontent-%COMP%_moving-gradient;background:-webkit-linear-gradient(160deg,var(--color-loading-background) 40%,var(--color-loading-background-contrast) 50%,var(--color-loading-background) 60%);background:linear-gradient(290deg,var(--color-loading-background) 40%,var(--color-loading-background-contrast) 50%,var(--color-loading-background) 60%);background-size:800px;border-radius:8px;height:24px;margin:2px 0}@-webkit-keyframes _ngcontent-%COMP%_moving-gradient{0%{background-position:-400px 0}to{background-position:400px 0}}@keyframes _ngcontent-%COMP%_moving-gradient{0%{background-position:-400px 0}to{background-position:400px 0}}"],
			data: { animation: [
				_.qm("popover", [_.um(":enter", [_.sm({
					opacity: 0,
					transform: "translateX(-8px)"
				}), _.rm("200ms cubic-bezier(0.2, 0, 0, 1)", _.sm({
					opacity: 1,
					transform: "translateX(0)"
				}))]), _.um(":leave", [_.rm("150ms cubic-bezier(0.4, 0, 1, 1)", _.sm({
					opacity: 0,
					transform: "translateX(-8px)"
				}))])]),
				_.qm("collapse", [_.um(":enter", [_.sm({
					height: 0,
					opacity: 0,
					overflow: "hidden"
				}), _.rm("200ms cubic-bezier(0.2, 0, 0, 1)", _.sm({
					height: "*",
					opacity: 1
				}))]), _.um(":leave", [_.sm({ overflow: "hidden" }), _.rm("150ms cubic-bezier(0.4, 0, 1, 1)", _.sm({
					height: 0,
					opacity: 0
				}))])]),
				_.qm("flyoutItem", [_.um(":enter", [_.sm({
					opacity: 0,
					height: 0,
					transform: "translateY(-4px)"
				}), _.rm("200ms 50ms cubic-bezier(0.2, 0, 0, 1)", _.sm({
					opacity: 1,
					height: "*",
					transform: "translateY(0)"
				}))]), _.um(":leave", [_.rm("150ms cubic-bezier(0.4, 0, 1, 1)", _.sm({
					opacity: 0,
					height: 0,
					transform: "translateY(-4px)"
				}))])])
			] }
		});
		var jud = [_.qm("parent", [_.um(":enter", [])]), _.qm("fadeInOut", [_.um(":enter", [_.sm({ opacity: 0 }), _.rm("200ms ease-in", _.sm({ opacity: 1 }))]), _.um(":leave", [_.rm("200ms ease-in", _.sm({ opacity: 0 }))])])];
		var nud = class {
			constructor() {
				this.S = _.Dk;
				this.Zv = _.Wy;
				this.Rsa = 5;
				this.ma = _.m(_.ZC);
				this.Kh = _.lp();
				this.U = this.ma.A.small;
				this.Id = _.W(() => this.Kh || this.U());
				this.fa = _.m(_.rF);
				this.ea = _.m(_.pG);
				this.A = _.m(c4);
				this.Ga = _.m(_.Jf);
				this.I = _.m(_.V3);
				this.Ia = _.m(_.oF);
				this.X = _.m(_.BF);
				this.m1a = _.Ni("historyTriggerLink");
				this.nU = _.hi();
				this.H = _.m(_.jH);
				this.Jf = _.m(_.UH);
				this.aa = _.m(_.Cl);
				this.vc = _.M(localStorage.getItem("is_playground_expanded") !== "false");
				this.NB = _.M(false);
				this.Wd = _.Ck(this.fa.Xm, { initialValue: undefined });
				this.Ye = Ztd;
				this.MI = mud;
				this.UM = _.W(() => this.I.UM().slice(0, 5));
				this.Sa = this.I.Sa;
				this.ve = {
					I2: 281714,
					Psa: 281710
				};
				this.cDa = _.W(() => {
					var b = this.ea.F;
					var c;
					var d;
					b = (d = (c = b().routeConfig) == null ? undefined : c.path) != null ? d : "";
					return b.startsWith("prompts") || b.startsWith("generate-speech") || b.startsWith("live");
				});
				this.R = _.W(() => {
					var b = this.Wd();
					return b && !this.Jf.Gx() || b && !this.Jf.U();
				});
				var a = true;
				_.Fk([this.R, this.H.F], () => {
					if (this.R() && (this.H.F() || a)) {
						a = false, _.Dod(this.I, this.aa.url !== "/library"), this.H.A.set(false);
					}
				});
				_.Fk([this.NB], (b) => {
					if (this.NB()) {
						let c = (d) => {
							d = d.target;
							var e;
							var f;
							if (!(this.Ga.nativeElement.contains(d) || (e = d.closest) != null && e.call(d, ".cdk-overlay-container")) || ((f = d.closest) == null ? 0 : f.call(d, "a"))) {
								this.close();
							}
						};
						document.addEventListener("click", c);
						b(() => {
							document.removeEventListener("click", c);
							this.Nr();
						});
					}
				});
			}
			bo(a) {
				this.vc.update((b) => !b);
				localStorage.setItem("is_playground_expanded", String(this.vc()));
				if (!this.vc()) {
					this.close();
				}
				a.preventDefault();
				a.stopPropagation();
			}
			togglePopover() {
				if (this.NB()) {
					this.close();
				} else {
					this.JI();
				}
			}
			JI() {
				this.Nr();
				this.A.A.set("playground-history");
				this.NB.set(true);
			}
			close() {
				this.NB.set(false);
				var a = this.A;
				if (a.A() === "playground-history") {
					a.A.set(null);
				}
			}
			QU() {
				this.Nr();
				this.F = setTimeout(() => {
					this.close();
				}, 300);
			}
			Nr() {
				if (this.F) {
					clearTimeout(this.F), this.F = undefined;
				}
			}
			y_() {
				X3(this.A, "sub-item");
			}
			Eu(a) {
				if (a.text === "Playground") {
					var b = this.Ia.promptModel();
					if (b && (b = _.AF(this.X, b)) && _.Mm(b)) {
						this.Ia.promptModel.set(undefined);
					}
				}
				X3(this.A, a.text);
			}
			vGa(a) {
				if (a.key === "ArrowRight" || a.key === "ArrowDown") {
					this.JI(), setTimeout(() => {
						var b = this.nU().map((c) => c.nativeElement);
						if (b.length > 0) {
							b[0].focus();
						}
					}, 0), a.preventDefault();
				} else {
					if (a.key === "Tab") {
						this.close();
					}
				}
			}
			fGa(a) {
				var b = this.nU().map((d) => d.nativeElement);
				if (b.length !== 0) {
					var c = b.indexOf(document.activeElement);
					if (a.key === "ArrowDown") {
						let d;
						if (!((d = b[(c + 1) % b.length]) == null)) {
							d.focus();
						}
						a.preventDefault();
					} else if (a.key === "ArrowUp") {
						let d;
						if (!((d = b[(c - 1 + b.length) % b.length]) == null)) {
							d.focus();
						}
						a.preventDefault();
					} else if (a.key === "Escape" || a.key === "Tab" || a.key === "ArrowLeft") {
						this.close();
						let d;
						if (!((d = this.m1a()) == null)) {
							d.nativeElement.focus();
						}
						a.preventDefault();
					}
				}
			}
		};
		nud.J = function(a) {
			return new (a || nud)();
		};
		nud.ka = _.u({
			type: nud,
			da: [["ms-nav-items-playground-v2"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.m1a, kud, 5)(b.nU, lud, 5);
				}
				if (a & 2) {
					_.ki(2);
				}
			},
			ha: 6,
			ia: 13,
			la: [
				[
					"historyTriggerLink",
					"",
					"historyTrigger",
					"cdkOverlayOrigin"
				],
				["selectableItem", ""],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					1,
					"playground-link",
					3,
					"click",
					"iconName",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions",
					"ve",
					"veImpression",
					"veClick"
				],
				[1, "nav-item-main-text"],
				[
					"role",
					"button",
					"tabindex",
					"0",
					1,
					"expand-chevron",
					3,
					"iconName",
					"expanded",
					"ve",
					"veImpression",
					"veClick"
				],
				[1, "sub-items"],
				[
					"role",
					"button",
					"tabindex",
					"0",
					1,
					"expand-chevron",
					3,
					"click",
					"keydown.enter",
					"keydown.space",
					"iconName",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					1,
					"sub-item",
					3,
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions"
				],
				[
					"cdkOverlayOrigin",
					"",
					1,
					"sub-item",
					3,
					"click",
					"mouseenter",
					"focus",
					"keydown",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions"
				],
				[
					"popoverId",
					"playground-history",
					3,
					"close",
					"panelKeydown",
					"containerElement",
					"isOpen",
					"triggerOrigin"
				],
				[1, "flyout-section-label"],
				[1, "flyout-list"],
				[1, "flyout-item"],
				[
					1,
					"flyout-item-link",
					3,
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions"
				],
				[1, "info-message"],
				[
					1,
					"flyout-item",
					"prompt-loading-shimmer"
				],
				[
					1,
					"sub-item",
					3,
					"click",
					"routerLink",
					"routerLinkActive",
					"routerLinkActiveOptions"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Th(0), _.F(1, "a", 2), _.J("click", function() {
						return b.Eu(b.MI);
					}), _.F(2, "span", 3), _.R(3), _.H(), _.B(4, Rqd, 1, 8, "span", 4), _.H(), _.B(5, brd, 3, 2, "div", 5);
				}
				if (a & 2) {
					_.Uh(b.UM()), _.y(), _.P("active-override", b.cDa()), _.E("iconName", b.MI.icon)("routerLink", b.MI.routerLink)("routerLinkActive", b.Ye.class)("routerLinkActiveOptions", b.Ye.options)("ve", b.ve.Psa)("veImpression", true)("veClick", true), _.y(2), _.U(b.MI.text), _.y(), _.C(b.Wd() ? 4 : -1), _.y(), _.C(b.Wd() && b.vc() ? 5 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				d4,
				_.HB,
				_.FB,
				_.sA,
				_.D3,
				_.Cz,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{padding-left:8px;padding-right:8px;border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]   .ms-button-icon-symbol[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:24px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.nav-item-main-text[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;text-align:left}.expand-chevron[_ngcontent-%COMP%]{-webkit-transition:-webkit-transform .2s ease;transition:-webkit-transform .2s ease;transition:transform .2s ease;transition:transform .2s ease,-webkit-transform .2s ease;cursor:pointer}.expand-chevron.expanded[_ngcontent-%COMP%]{-webkit-transform:rotate(90deg);transform:rotate(90deg)}.expand-chevron[_ngcontent-%COMP%]:focus-visible{outline:auto}.sub-items[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;padding-top:2px;margin-left:20px;padding-left:12px;border-left:1px solid light-dark(rgba(0,0,0,.12),hsla(0,0%,100%,.12))}.sub-item[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;height:32px;padding:0 12px 0 8px;border-radius:12px;text-decoration:none;color:var(--color-v3-text-var);cursor:pointer;-webkit-transition:background-color .15s ease-in-out;transition:background-color .15s ease-in-out}.sub-item[_ngcontent-%COMP%]:hover{background:var(--color-nav-item-hover);color:var(--color-v3-text)}.sub-item.active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.flyout-section-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:light-dark(rgba(0,0,0,.4),hsla(0,0%,100%,.4));padding:6px 8px 2px;text-transform:uppercase;letter-spacing:.4px}.flyout-list[_ngcontent-%COMP%]{list-style:none;margin:0;padding:0;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:1px}.flyout-item[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;height:32px;padding:0 8px;border-radius:10px;cursor:pointer;gap:8px;-webkit-transition:background-color .12s ease;transition:background-color .12s ease}.flyout-item[_ngcontent-%COMP%]:focus-within, .flyout-item[_ngcontent-%COMP%]:hover{background:light-dark(rgba(0,0,0,.05),hsla(0,0%,100%,.08))}.flyout-item[_ngcontent-%COMP%]:focus-within   .flyout-item-link[_ngcontent-%COMP%], .flyout-item[_ngcontent-%COMP%]:hover   .flyout-item-link[_ngcontent-%COMP%]{color:var(--color-v3-text)}.flyout-item-link[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:light-dark(#5b5b64,hsla(0,0%,100%,.65));text-decoration:none;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.flyout-item-link[_ngcontent-%COMP%]:focus-visible{outline:auto;outline-offset:2px}.info-message[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;padding:8px;color:light-dark(#5b5b64,hsla(0,0%,100%,.65));margin:0}.prompt-loading-shimmer[_ngcontent-%COMP%]{-webkit-animation-duration:1.7s;animation-duration:1.7s;-webkit-animation-fill-mode:forwards;animation-fill-mode:forwards;-webkit-animation-iteration-count:infinite;animation-iteration-count:infinite;-webkit-animation-timing-function:linear;animation-timing-function:linear;-webkit-animation-name:_ngcontent-%COMP%_moving-gradient;animation-name:_ngcontent-%COMP%_moving-gradient;background:-webkit-linear-gradient(160deg,var(--color-loading-background) 40%,var(--color-loading-background-contrast) 50%,var(--color-loading-background) 60%);background:linear-gradient(290deg,var(--color-loading-background) 40%,var(--color-loading-background-contrast) 50%,var(--color-loading-background) 60%);background-size:800px;border-radius:8px;height:24px;margin:2px 0}@-webkit-keyframes _ngcontent-%COMP%_moving-gradient{0%{background-position:-400px 0}to{background-position:400px 0}}@keyframes _ngcontent-%COMP%_moving-gradient{0%{background-position:-400px 0}to{background-position:400px 0}}"],
			data: { animation: [
				...jud,
				_.qm("popover", [_.um(":enter", [_.sm({
					opacity: 0,
					transform: "translateX(-8px)"
				}), _.rm("200ms cubic-bezier(0.2, 0, 0, 1)", _.sm({
					opacity: 1,
					transform: "translateX(0)"
				}))]), _.um(":leave", [_.rm("150ms cubic-bezier(0.4, 0, 1, 1)", _.sm({
					opacity: 0,
					transform: "translateX(-8px)"
				}))])]),
				_.qm("collapse", [_.um(":enter", [_.sm({
					height: 0,
					opacity: 0,
					overflow: "hidden"
				}), _.rm("200ms cubic-bezier(0.2, 0, 0, 1)", _.sm({
					height: "*",
					opacity: 1
				}))]), _.um(":leave", [_.sm({ overflow: "hidden" }), _.rm("150ms cubic-bezier(0.4, 0, 1, 1)", _.sm({
					height: 0,
					opacity: 0
				}))])])
			] }
		});
		var oud = function(a) {
			if (a.R) {
				var b = _.kd(_.qA(a.H, a.H.bk(["spend"])));
				let c = _.H3(b);
				let d = _.G3(b);
				b = _.F3(b);
				let e = _.I3();
				if (!(_.N3(a.A, c) || _.N3(a.A, d) || _.N3(a.A, b) || _.N3(a.A, e))) {
					setTimeout(() => {
						a.A.Fi.next(c);
					}, 1e3);
				}
			}
		};
		var pud = class {
			constructor() {
				this.Qc = _.m(_.BM);
				this.U = _.m(c4);
				this.H = _.m(_.Cl);
				this.A = _.m(_.O3);
				this.F = _.m(_.Op);
				this.X = _.m(_.pG);
				this.I = _.m(_.yG);
				this.R = this.F.getFlag(_.iL);
				this.gY = this.F.getFlag(_.Wzb);
				this.fya = "documentation";
				this.Jya = "https://ai.google.dev/gemini-api/docs";
				this.Ye = Ztd;
				this.S = _.Dk;
				this.Zra = "dashboard-nav-item";
				this.s2 = {
					text: "Build",
					icon: "design_services",
					routerLink: "apps"
				};
				this.eya = this.X.I;
				this.hG = {
					text: "Dashboard",
					icon: "speed"
				};
				this.ve = {
					tea: 281711,
					Nsa: 281712,
					H2: 281713
				};
				_.Fk([this.Qc.isNavbarExpanded, this.I.A], () => {
					if (this.Qc.isNavbarExpanded() && this.I.A()) {
						oud(this);
					} else {
						_.M3(this.A);
					}
				});
			}
			Eu(a) {
				X3(this.U, a.text, a.text !== this.hG.text);
			}
		};
		pud.J = function(a) {
			return new (a || pud)();
		};
		pud.ka = _.u({
			type: pud,
			da: [["ms-nav-items-main-v2"]],
			ha: 8,
			ia: 9,
			la: [
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"click",
					"xapTourElementId",
					"iconName",
					"routerLink",
					"ve",
					"veImpression",
					"veClick"
				],
				[1, "nav-item-main-text"],
				[3, "iconName"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"iconName",
					"routerLink",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"target",
					"_blank",
					3,
					"iconName",
					"href",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					1,
					"documentation-outward-icon",
					3,
					"iconName"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.I(0, "ms-nav-items-playground-v2")(1, "ms-nav-items-build-v2"), _.F(2, "a", 0), _.J("click", function() {
						return b.Eu(b.hG);
					}), _.F(3, "span", 1), _.R(4), _.H(), _.I(5, "span", 2), _.H(), _.B(6, crd, 3, 5, "a", 3)(7, drd, 4, 6, "a", 4);
				}
				if (a & 2) {
					_.y(2), _.E("xapTourElementId", b.Zra)("iconName", b.hG.icon)("routerLink", b.eya())("ve", b.ve.Nsa)("veImpression", true)("veClick", true), _.y(2), _.S(" ", b.hG.text, " "), _.y(), _.E("iconName", b.S.gh), _.y(), _.C(b.gY ? 6 : 7);
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				iud,
				nud,
				_.sA,
				_.Cz,
				_.Bz,
				_.P3
			],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{padding-left:8px;padding-right:8px;border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]   .ms-button-icon-symbol[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:24px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}[_nghost-%COMP%]{gap:2px}.nav-item-main-text[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;text-align:left}"]
		});
		var qud = new Map([
			[0, {
				title: "",
				route: "prompts"
			}],
			[1, {
				title: "Dashboard",
				route: "apikey"
			}],
			[2, {
				title: "Build",
				route: "apps"
			}]
		]);
		var rud = class {
			constructor() {
				this.H = _.m(_.pG);
				this.F = _.m(c4);
				this.bK = ftd;
				this.S = _.Dk;
				this.ve = { Osa: 281715 };
				this.KKa = _.W(() => qud.get(this.ck()).title);
				this.A = _.W(() => {
					var a;
					var b;
					var c;
					return (c = (a = this.H.F()) == null ? undefined : (b = a.routeConfig) == null ? undefined : b.path) != null ? c : "";
				});
				this.ck = _.W(() => {
					var a = this.A();
					return this.F.F.some((b) => b.routerLink === a) ? 1 : 0;
				});
				this.qEa = _.W(() => "/prompts/new_chat");
			}
		};
		rud.J = function(a) {
			return new (a || rud)();
		};
		rud.ka = _.u({
			type: rud,
			da: [["ms-nav-items-v2"]],
			ha: 3,
			ia: 2,
			la: [[
				"ms-button",
				"",
				"variant",
				"borderless",
				1,
				"back-item",
				3,
				"iconName",
				"routerLink",
				"ve",
				"veClick",
				"veImpression"
			], [1, "nav_items_nav_mode"]],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, erd, 2, 6, "a", 0), _.B(1, frd, 1, 0, "ms-nav-items-main-v2", 1)(2, grd, 1, 0, "ms-nav-items-dashboard-v2", 1);
				}
				if (a & 2) {
					a = b.ck(), _.C(a !== b.bK.uea ? 0 : -1), _.y(), _.C(a === b.bK.uea ? 1 : a === b.bK.DASHBOARD ? 2 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.IC,
				bud,
				pud,
				_.sA,
				_.Cz,
				_.Bz
			],
			styles: ["ul[_ngcontent-%COMP%]{list-style-type:none;margin:0;padding:0}[ms-button][_ngcontent-%COMP%]{padding-left:8px;padding-right:8px;border-radius:12px}[ms-button][_ngcontent-%COMP%]:not([variant=icon-borderless]){width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start}[ms-button][_ngcontent-%COMP%]   .ms-button-icon-symbol[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:24px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}[ms-button][_ngcontent-%COMP%]:not(.active):not(.active-override){color:var(--color-v3-text-var)}[ms-button][_ngcontent-%COMP%]:hover:not(.active):not(.active-override){background:var(--color-nav-item-hover);color:var(--color-v3-text)}[ms-button].active[_ngcontent-%COMP%]{background:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}.active-override[_ngcontent-%COMP%]{background-color:var(--color-nav-item-active);color:var(--color-v3-text);font-weight:500}[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}.back-item[_ngcontent-%COMP%]{margin-bottom:4px}.nav_items_nav_mode[_ngcontent-%COMP%]{opacity:1}@-webkit-keyframes _ngcontent-%COMP%_slideInFromLeft{0%{-webkit-transform:translateX(-100%);transform:translateX(-100%);opacity:0}to{-webkit-transform:translateX(0);transform:translateX(0);opacity:1}}@keyframes _ngcontent-%COMP%_slideInFromLeft{0%{-webkit-transform:translateX(-100%);transform:translateX(-100%);opacity:0}to{-webkit-transform:translateX(0);transform:translateX(0);opacity:1}}.slide-in-from-left[_ngcontent-%COMP%]{-webkit-animation:_ngcontent-%COMP%_slideInFromLeft .7s cubic-bezier(.23,1,.32,1);animation:_ngcontent-%COMP%_slideInFromLeft .7s cubic-bezier(.23,1,.32,1)}@-webkit-keyframes _ngcontent-%COMP%_slideInFromRight{0%{-webkit-transform:translateX(100%);transform:translateX(100%);opacity:0}to{-webkit-transform:translateX(0);transform:translateX(0);opacity:1}}@keyframes _ngcontent-%COMP%_slideInFromRight{0%{-webkit-transform:translateX(100%);transform:translateX(100%);opacity:0}to{-webkit-transform:translateX(0);transform:translateX(0);opacity:1}}.slide-in-from-right[_ngcontent-%COMP%]{-webkit-animation:_ngcontent-%COMP%_slideInFromRight .7s cubic-bezier(.23,1,.32,1);animation:_ngcontent-%COMP%_slideInFromRight .7s cubic-bezier(.23,1,.32,1)}"]
		});
		var vud = function(a) {
			if (a.fa) {
				let b = _.kd(_.qA(a.H, a.H.bk(["spend"])));
				let c = _.H3(b);
				let d = _.G3(b);
				let e = _.I3();
				let f = _.F3(b);
				if (!(_.N3(a.A, c) || _.N3(a.A, d) || _.N3(a.A, e) || _.N3(a.A, f))) {
					setTimeout(() => {
						if (!a.Qc.isNavbarExpanded()) {
							a.A.Fi.next(f);
						}
					}, 1e3);
				}
			}
		};
		var wud = class {
			constructor() {
				this.F = _.m(_.Ou);
				this.ma = _.m(_.pG);
				this.X = _.m(_.Qu);
				this.Qc = _.m(_.BM);
				_.m(c4);
				this.D4 = _.m(_.s3);
				this.Vb = _.m(_.AG);
				this.na = _.m(_.OC);
				this.A = _.m(_.O3);
				this.H = _.m(_.Cl);
				this.U = _.m(_.Op);
				this.Ig = _.m(_.C3);
				this.I = _.m(_.yG);
				this.fa = this.U.getFlag(_.iL);
				this.Aea = "nav-button";
				this.V3 = _.Ni(Y3);
				this.n$ = _.Ni("msAccountSwitcher");
				this.Ge = _.Jp;
				this.S = _.Dk;
				this.rxa = `${_.Aa() ? "⌘" : "Ctrl"} /`;
				this.ve = { Ojb: 262630 };
				this.Oe = this.X.Oe;
				this.url = this.ma.url;
				this.vHa = sud;
				this.R = [
					"prompts",
					"live",
					"generate-speech",
					"new_music"
				];
				this.PJa = _.W(() => {
					if (!this.url()) return true;
					var a = this.url().split("?")[0];
					return !(a === "/" || this.R.some((b) => a.startsWith(`/${b}`)));
				});
				this.nEa = _.W(() => {
					var a = this.url();
					return (a ? a.split("?")[0] : "").startsWith("/apps/") ? "/apps" : "/prompts/new_chat";
				});
				this.yf = this.Vb.yf;
				this.Bu = this.Vb.Bu;
				this.aa = this.Vb.tj;
				this.Hb = _.W(() => _.Nn(this.Oe));
				this.QJa = _.W(() => this.Ge() && this.aa() && !this.yf() && !this.Hb() && !this.bb() && this.ea());
				this.Le = this.Vb.Le;
				this.bb = this.na.bb;
				this.Uba = _.W(() => this.Le() && this.yf() || !!this.bb());
				this.V9 = _.W(() => {
					var a;
					var b;
					return ((a = this.bb()) == null ? undefined : (b = _.Io(a)) == null ? undefined : b.slice(-4)) || "";
				});
				this.o9 = _.W(() => _.qp(this.url()));
				this.ea = _.W(() => this.o9() || this.url().startsWith("/apps"));
				_.Fk([this.Qc.isNavbarExpanded, this.I.A], () => {
					if (!this.Qc.isNavbarExpanded() && this.I.A()) {
						vud(this);
					} else {
						_.M3(this.A);
					}
				});
			}
			iu() {
				_.Rn(this.F, "NAV", "Toggled Nav");
				this.Qc.iu();
			}
			kxa() {
				_.Rn(this.F, "NAV", "Clicked Logo");
			}
			jxa(a) {
				a.stopPropagation();
				if (this.Ge()) {
					let b;
					if (!((b = this.n$()) == null)) {
						b.nO();
					}
				} else {
					let b;
					if (!((b = this.V3()) == null)) {
						b.nO();
					}
				}
			}
			HGa() {
				this.Ig.open(2);
			}
		};
		wud.J = function(a) {
			return new (a || wud)();
		};
		wud.ka = _.u({
			type: wud,
			da: [["ms-navbar-v2"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.V3, Y3, 5)(b.n$, tud, 5);
				}
				if (a & 2) {
					_.ki(2);
				}
			},
			ha: 30,
			ia: 15,
			la: [
				["productsMenu", "matMenu"],
				["accountSwitcher", ""],
				["msAccountSwitcher", ""],
				[
					"ms-button",
					"",
					"variant",
					"icon-primary",
					"matTooltip",
					"Toggle navigation menu",
					"aria-label",
					"Toggle navigation menu",
					1,
					"floating-toggle-button",
					3,
					"xapTourElementId",
					"expanded",
					"banner-visible",
					"iconName"
				],
				[
					1,
					"nav-content",
					"v3-left-nav",
					"nesting-enabled"
				],
				[
					1,
					"navbar-header",
					"v3-design"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					1,
					"logo-wrapper",
					"logo-link",
					3,
					"click",
					"routerLink"
				],
				["type", "lockup"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"View related products",
					1,
					"dropdown-trigger",
					3,
					"iconName",
					"matMenuTriggerFor"
				],
				[1, "products-menu"],
				[1, "products-menu-header"],
				[
					"mat-menu-item",
					"",
					"target",
					"_blank",
					"rel",
					"noopener noreferrer",
					1,
					"product-menu-item",
					3,
					"href"
				],
				[1, "empty-space"],
				[1, "bottom-actions"],
				[3, "isNavbarExpanded"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"matTooltipPosition",
					"right",
					1,
					"command-palette-button",
					3,
					"click",
					"iconName"
				],
				[1, "label"],
				[1, "shortcut"],
				[3, "ngTemplateOutlet"],
				[
					"ms-button",
					"",
					"variant",
					"icon-primary",
					"matTooltip",
					"Toggle navigation menu",
					"aria-label",
					"Toggle navigation menu",
					1,
					"floating-toggle-button",
					3,
					"click",
					"xapTourElementId",
					"iconName"
				],
				[1, "product-icon-container"],
				[
					1,
					"product-icon",
					3,
					"iconName"
				],
				[1, "product-content"],
				[1, "product-name"],
				[1, "product-description"],
				[
					1,
					"external-link-icon",
					3,
					"iconName"
				],
				[1, "account-switcher-container"],
				[
					"ms-button",
					"",
					1,
					"account-switcher-button",
					3,
					"click",
					"variant"
				],
				[1, "avatar-placeholder"],
				[1, "account-switcher-text"],
				[
					1,
					"navbar-badge",
					"hide-circle"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Th(0), _.B(1, hrd, 1, 7, "button", 3), _.F(2, "div", 4)(3, "div", 5)(4, "a", 6), _.J("click", function() {
						return b.kxa();
					}), _.I(5, "ms-logo-icon", 7), _.H(), _.I(6, "button", 8), _.H(), _.F(7, "mat-menu", 9, 0)(9, "div", 10), _.R(10, "Products and apps"), _.H(), _.Ah(11, ird, 9, 5, "a", 11, uud), _.H(), _.F(13, "nav"), _.I(14, "ms-nav-items-v2")(15, "div", 12), _.F(16, "div", 13), _.I(17, "ms-navbar-disclaimer", 14), _.B(18, lrd, 3, 0), _.F(19, "button", 15), _.J("click", function() {
						return b.HGa();
					}), _.F(20, "span", 16), _.R(21, "Search"), _.H(), _.F(22, "span", 17), _.R(23), _.H()(), _.I(24, "ms-updates")(25, "ms-api-key-button")(26, "ms-settings-menu"), _.Ih(27, 18), _.H()()(), _.z(28, wrd, 6, 7, "ng-template", null, 1, _.Ii);
				}
				if (a & 2) {
					a = _.O(8);
					let c = _.O(29);
					let d = _.Uh(b.Qc.isNavbarExpanded());
					_.y();
					_.C(b.PJa() ? 1 : -1);
					_.y();
					_.P("expanded", d)("collapsed", !d);
					_.E("@parent", undefined);
					_.y(2);
					_.E("routerLink", b.nEa());
					_.y(2);
					_.E("iconName", b.S.Ck)("matMenuTriggerFor", a);
					_.y(5);
					_.Bh(b.vHa);
					_.y(6);
					_.E("isNavbarExpanded", d);
					_.y();
					_.C(b.QJa() ? 18 : -1);
					_.y();
					_.E("iconName", b.S.Lm);
					_.y(4);
					_.U(b.rxa);
					_.y(4);
					_.E("ngTemplateOutlet", c);
				}
			},
			dependencies: [
				Y3,
				Z3,
				_.Yy,
				_.tz,
				_.nz,
				_.dz,
				_.tG,
				_.OD,
				_.OE,
				_.$D,
				_.yA,
				_.fF,
				_.wI,
				_.tI,
				_.sI,
				_.vI,
				_.IC,
				_.HC,
				rud,
				b4,
				_.sA,
				$3,
				a4,
				_.Cz,
				_.P3
			],
			styles: [".nav-content[_ngcontent-%COMP%]{-webkit-user-select:none;-moz-user-select:none;-ms-user-select:none;user-select:none;background-color:var(--color-v3-surface-left-nav);border-right:1px solid var(--color-v3-surface-left-nav-border);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;height:100%;overflow:hidden auto;padding:0 18px;scrollbar-gutter:unset;-webkit-transition:width .2s cubic-bezier(.4,0,.2,0),opacity .2s cubic-bezier(.4,0,.2,0),left .2s cubic-bezier(.4,0,.2,0),padding .2s cubic-bezier(.4,0,.2,0);transition:width .2s cubic-bezier(.4,0,.2,0),opacity .2s cubic-bezier(.4,0,.2,0),left .2s cubic-bezier(.4,0,.2,0),padding .2s cubic-bezier(.4,0,.2,0);z-index:5}.nav-content.collapsed[_ngcontent-%COMP%]{width:80px}.nav-content.collapsed.v3-left-nav[_ngcontent-%COMP%]{width:0;padding:0}.nav-content.expanded[_ngcontent-%COMP%]{width:220px}.nav-content.nesting-enabled[_ngcontent-%COMP%]{padding-left:8px;padding-right:8px}@media screen and (max-width:960px){.nav-content[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container);bottom:0;left:0;position:fixed;top:0;-webkit-transform:translate(-100%);transform:translate(-100%);-webkit-transition:opacity .2s ease-out,-webkit-transform .2s ease-out;transition:opacity .2s ease-out,-webkit-transform .2s ease-out;transition:transform .2s ease-out,opacity .2s ease-out;transition:transform .2s ease-out,opacity .2s ease-out,-webkit-transform .2s ease-out}.nav-content.expanded[_ngcontent-%COMP%]{opacity:1;-webkit-transform:translate(0);transform:translate(0);-webkit-transition:opacity .2s ease-in,-webkit-transform .2s ease-in;transition:opacity .2s ease-in,-webkit-transform .2s ease-in;transition:transform .2s ease-in,opacity .2s ease-in;transition:transform .2s ease-in,opacity .2s ease-in,-webkit-transform .2s ease-in}}.floating-toggle-button[_ngcontent-%COMP%]{position:absolute;top:16px;left:20px;-webkit-transition:translate .2s cubic-bezier(.4,0,.2,0);transition:translate .2s cubic-bezier(.4,0,.2,0);z-index:3}.floating-toggle-button.banner-visible[_ngcontent-%COMP%]{top:76px}@media screen and (max-width:600px){.floating-toggle-button[_ngcontent-%COMP%]{translate:-8px}}@media screen and (min-width:769px){.floating-toggle-button.expanded[_ngcontent-%COMP%]{translate:220px 0}}.navbar-header[_ngcontent-%COMP%]{position:-webkit-sticky;position:sticky;top:0;z-index:2;background:inherit}.navbar-header.v3-design[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;padding:16px 0;width:100%}.navbar-header[_ngcontent-%COMP%]   .logo-wrapper[_ngcontent-%COMP%]{width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;padding:0 8px;text-decoration:none;color:inherit}.navbar-header[_ngcontent-%COMP%]   .logo-wrapper[_ngcontent-%COMP%]   ms-logo-icon[_ngcontent-%COMP%]{max-width:140px}.navbar-header[_ngcontent-%COMP%]   .dropdown-trigger[_ngcontent-%COMP%]{color:var(--color-v3-text-var);margin-left:auto}nav[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:1;-webkit-flex:1 0;-moz-box-flex:1;-ms-flex:1 0;flex:1 0;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}.computer-use-case-nav-item[_ngcontent-%COMP%]{padding:10px 8px;margin-block:2px}.computer-use-case-nav-item[_ngcontent-%COMP%]   a[_ngcontent-%COMP%]{cursor:pointer}.computer-use-case-nav-item[_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.empty-space[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1}.empty-space.clickable[_ngcontent-%COMP%]{margin-inline:-18px;cursor:pointer}.bottom-actions[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:2px;padding-bottom:16px}.bottom-actions[_ngcontent-%COMP%] > [ms-button][_ngcontent-%COMP%]{color:var(--color-v3-text-var);-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;width:100%;border-radius:12px;padding-left:8px;padding-right:8px}.bottom-actions[_ngcontent-%COMP%] > [ms-button][_ngcontent-%COMP%]   .ms-button-icon-symbol[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:24px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.bottom-actions[_ngcontent-%COMP%] > [ms-button][_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.bottom-actions[_ngcontent-%COMP%]   ms-navbar-disclaimer[_ngcontent-%COMP%]{margin-bottom:4px}.bottom-actions[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%], .bottom-actions[_ngcontent-%COMP%]   .command-palette-button[_ngcontent-%COMP%]{-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;width:100%}.bottom-actions[_ngcontent-%COMP%]   .command-palette-button[_ngcontent-%COMP%]   .shortcut[_ngcontent-%COMP%]{color:var(--color-v3-text-var);margin-left:auto;padding-left:4px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;opacity:0;-webkit-transition:opacity .15s ease;transition:opacity .15s ease}.bottom-actions[_ngcontent-%COMP%]   .command-palette-button[_ngcontent-%COMP%]:hover   .shortcut[_ngcontent-%COMP%]{opacity:1}.account-switcher-container[_ngcontent-%COMP%]{position:relative}.account-switcher-container[_ngcontent-%COMP%]:has(.v3-container-485387979){height:48px}.account-switcher-container[_ngcontent-%COMP%]   alkali-accountswitcher[_ngcontent-%COMP%]{position:absolute;left:12px;top:8px}.account-switcher-container[_ngcontent-%COMP%]   ms-account-switcher[_ngcontent-%COMP%]{position:absolute;left:12px;top:16px}.account-switcher-container[_ngcontent-%COMP%]   .avatar-placeholder[_ngcontent-%COMP%]{width:18px;-webkit-box-flex:0;-webkit-flex:0 0 18px;-moz-box-flex:0;-ms-flex:0 0 18px;flex:0 0 18px}.account-switcher-container[_ngcontent-%COMP%]   .avatar-placeholder.g1-member[_ngcontent-%COMP%]{width:24px;-webkit-box-flex:0;-webkit-flex:0 0 24px;-moz-box-flex:0;-ms-flex:0 0 24px;flex:0 0 24px}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%]{width:100%;-webkit-box-pack:start;-webkit-justify-content:start;-moz-box-pack:start;-ms-flex-pack:start;justify-content:start;color:var(--color-v3-text-var);padding-left:8px;padding-right:8px}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%]:hover{background-color:var(--color-nav-item-hover)}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button.v3-container-485387979[_ngcontent-%COMP%]{border:1px solid var(--color-v3-outline-var);background-color:var(--color-v3-surface-container);height:auto;margin-top:4px;padding:8px 12px}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button.v3-container-485387979[_ngcontent-%COMP%]:hover{background-color:var(--color-v3-hover)}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button.v3-container-485387979[_ngcontent-%COMP%]   .account-switcher-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var);line-height:26px}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%]   .account-switcher-text[_ngcontent-%COMP%]{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}.account-switcher-container[_ngcontent-%COMP%]   .account-switcher-button[_ngcontent-%COMP%]   .navbar-badge[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;background-color:var(--color-v3-button-container);border-radius:8px;padding:4px 8px;margin-left:4px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;text-transform:uppercase}  .products-menu.mat-mdc-menu-panel{border-radius:12px;border:none;box-shadow:var(--v3-shadow-sm);background-color:var(--color-v3-surface-container);min-width:320px;overflow:visible}  .products-menu .mat-mdc-menu-content{padding:4px;background:none}  .products-menu .products-menu-header{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;font-weight:500;color:var(--color-v3-text-var);padding:8px 8px 4px;margin-left:0}  .products-menu.mat-mdc-menu-panel div.mat-mdc-menu-content a.product-menu-item.mat-mdc-menu-item{border-radius:12px}  .products-menu.mat-mdc-menu-panel div.mat-mdc-menu-content a.product-menu-item.mat-mdc-menu-item .product-icon.material-symbols-outlined{margin:0}  .products-menu .product-menu-item{border-radius:12px;padding:8px;-webkit-transition:background-color .15s cubic-bezier(.4,0,.2,1),color .15s cubic-bezier(.4,0,.2,1);transition:background-color .15s cubic-bezier(.4,0,.2,1),color .15s cubic-bezier(.4,0,.2,1)}  .products-menu .product-menu-item:hover{background-color:var(--color-v3-hover)}  .products-menu .product-menu-item .mat-mdc-menu-item-text{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;width:100%}  .products-menu .product-menu-item .product-icon-container{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:32px;height:32px;border-radius:12px;background-color:var(--color-v3-hover);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}  .products-menu .product-menu-item .product-icon{font-size:16px;line-height:1;color:var(--color-v3-text-var)}  .products-menu .product-menu-item .product-content{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:2px;-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;min-width:0}  .products-menu .product-menu-item .product-name{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text)}  .products-menu .product-menu-item .product-description{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}  .products-menu .product-menu-item .external-link-icon{font-size:16px;color:var(--color-v3-text-var);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;opacity:0;-webkit-transition:opacity .15s ease;transition:opacity .15s ease}  .products-menu .product-menu-item:hover .external-link-icon{opacity:1}"],
			data: { animation: jud }
		});
		var e4 = class {
			constructor() {
				this.A = _.m(_.xFb);
			}
		};
		e4.J = function(a) {
			return new (a || e4)();
		};
		e4.sa = _.Cd({
			token: e4,
			factory: e4.J,
			wa: "root"
		});
		var Xrd = (a) => ({ V: a });
		;
		var Urd = function(a) {
			return Array.from({ length: a != null ? a : 1 }, () => 0);
		};
		var esd = function(a, b) {
			if (b.variant === "command") {
				a.feedback.set({
					message: b.lv || `Executing ${b.label}...`,
					type: "loading"
				});
				b.action();
				a.feedback.set({
					message: b.successMessage || `${b.label} complete`,
					type: "success"
				});
				a.feedback.set(null);
				if (!b.Jpa) {
					a.close();
				}
			} else {
				var c;
				_.Qy(a.veLoggingService, 305482, _.Tmd(new _.Ky(), Ksd(gsd(hsd(isd(_.y3(new _.z3(), a.gz() ? 1 : 2), b.id), jsd(b.variant)), b.category), (c = a.Um()) == null ? undefined : c.title)));
				b.action();
				if (b.variant === "menu") {
					a.selectedIndex.set(0);
				}
				if (b.variant === "menu" || b.Jpa) {
					a.YD();
				} else {
					a.close();
				}
			}
		};
		var ksd = function(a) {
			return [
				a.category,
				a.label,
				a.description,
				a.D9 ? `Shortcut: ${a.D9.join("+")}` : "",
				a.variant === "menu" ? "Opens sub-menu" : ""
			].filter((b) => b).join(". ");
		};
		var Dud = function(a) {
			_.x(function* () {
				if (yield a.A.Wd()) yield _.Dod(a.A, true, false);
			});
		};
		var Eud = function(a) {
			a.stack.update((b) => b && b.length > 1 ? b.slice(0, -1) : b);
			_.Wmd(a.Ig);
			a.selectedIndex.set(0);
		};
		var Fud = function(a, b) {
			_.x(function* () {
				var c = a.Um();
				if (c && c.wj) {
					a.feedback.set({
						message: c.lv || "Processing...",
						type: "loading"
					});
					_.Qy(a.veLoggingService, 305483, _.Tmd(new _.Ky(), Lsd(Ksd(_.y3(new _.z3(), a.gz() ? 1 : 2), c.title), b.length)));
					try {
						let d = yield c.wj(b);
						a.feedback.set({
							message: d || "Done",
							type: "success"
						});
						a.feedback.set(null);
						a.close();
					} catch (d) {
						a.feedback.set({
							message: "Error",
							type: "success"
						});
						setTimeout(() => {
							a.feedback.set(null);
						}, 1500);
						console.error("Omnibar submit error:", d);
					}
				}
			});
		};
		var jsd = function(a) {
			switch (a) {
				case "command": return 1;
				case "navigation": return 2;
				case "menu": return 3;
				default: return 0;
			}
		};
		var Gud = class {
			constructor() {
				this.S = _.Dk;
				this.F = _.m(_.Cl);
				this.Ig = _.m(_.C3);
				this.Ga = _.m(_.Jf);
				this.Va = _.m(_.nI);
				this.A = _.m(_.V3);
				this.I = _.m(_.x3);
				this.Mj = _.m(_.SA);
				this.veLoggingService = _.m(_.Ry);
				this.Fka = _.Ni("inputField");
				this.Z8a = _.Ni("resultsList");
				this.DDa = _.hi();
				this.Tc = this.Ig.Tc;
				this.stack = this.Ig.stack;
				this.selectedIndex = _.M(-1);
				this.Rja = _.M(false);
				this.feedback = _.M(null);
				this.gz = _.V(false);
				this.ve = {
					Ynb: 305473,
					Znb: 305474,
					aob: 305475,
					bob: 305476,
					cob: 305477,
					dob: 305478,
					eob: 305479
				};
				this.U = _.W(() => {
					var a = this.stack();
					return a && a.length !== 0 ? a.length !== 1 || this.Ig.query() ? false : this.Va.Cm.Sa() || this.A.Sa() : false;
				});
				this.I0 = _.W(() => this.Tc() ? this.gz() ? this.Ig.I() === "overlay" : this.Ig.I() === "inline" : false);
				this.yGb = _.W(() => {
					var a = this.stack();
					var b = this.Um();
					if (!a || !b) return "Command Palette";
					a = a.slice(1).map((c) => {
						var d;
						return ((d = c.xf) == null ? undefined : d.label) || c.title;
					}).join(", ");
					b = this.Tc() ? b.placeholder : b.title;
					return a ? `${a}. ${b}` : b;
				});
				this.FLb = _.W(() => {
					var a = this.I.A().get("open-omnibar-overlay");
					if (!a) return [];
					var b = a.keys;
					var c = a.St;
					a = _.Lmd(b);
					b = _.Lmd(c || b, true);
					return _.Aa() ? b : a;
				});
				this.R = _.W(() => this.Va.Cm.xc() ? this.Va.Cm.value() : []);
				this.X = _.W(() => this.A.UM());
				this.H = _.W(() => {
					if (this.U()) return [{
						id: "recent-loading",
						label: "",
						variant: "command",
						action: () => {},
						category: "Recent activity",
						Sa: true,
						GJb: 3
					}];
					var a = this.R().slice(0, 3).map((c) => {
						var d;
						var e;
						var f;
						var g;
						var k;
						var p;
						var r;
						var v;
						return {
							id: `recent-applet-${(r = (d = c.Tf()) == null ? undefined : (e = _.Wo(d)) == null ? undefined : e.getId()) != null ? r : (f = c.Tf()) == null ? undefined : (g = _.So(f)) == null ? undefined : g.getResourceId()}`,
							label: c.getName(),
							description: "Vibe coding session",
							icon: "design_services",
							variant: "navigation",
							action: () => {
								this.F.navigate([_.r3(c.Tf())], { queryParams: _.wmd(c.Tf()) });
							},
							category: "Recent activity",
							LDa: (v = (k = _.Z(c, _.Zo, 1)) == null ? undefined : (p = k.toDate()) == null ? undefined : p.getTime()) != null ? v : 0
						};
					});
					var b = this.X().slice(0, 3).map((c) => {
						var d;
						return {
							id: `recent-chat-${c.id}`,
							label: c.text,
							description: "Chat session",
							icon: "chat_bubble",
							variant: "navigation",
							action: () => {
								this.F.navigate([c.routerLink]);
							},
							category: "Recent activity",
							LDa: (d = c.lastModified) != null ? d : 0
						};
					});
					return [...a, ...b].sort((c, d) => d.LDa - c.LDa);
				});
				this.Um = _.W(() => {
					var a = this.stack();
					return a && a.length > 0 ? a[a.length - 1] : undefined;
				});
				this.aR = _.W(() => btd(this.Ig, this.stack()));
				_.Fk([this.Tc], () => {
					if (this.Tc()) {
						setTimeout(() => {
							this.YD();
						});
					}
				});
				_.Fk([this.aR, this.Tc], () => {
					if (this.Tc()) {
						var a;
						if (((a = this.Um()) == null ? undefined : a.source) !== null) {
							a = this.aR().length, _.RA(this.Mj, a === 0 ? "No results found" : `${a} result${a === 1 ? "" : "s"} available`);
						}
					}
				});
				_.Fk([this.selectedIndex, this.DDa], () => {
					var a = this.selectedIndex();
					var b = this.DDa();
					var c;
					var d = (c = this.Z8a()) == null ? undefined : c.nativeElement;
					if (!(!d || !b || a < 0 || a >= b.length)) {
						var e;
						a = (e = b[a]) == null ? undefined : e.nativeElement;
						if ((e = a.previousElementSibling) && e.classList.contains("category-header") && e.offsetTop < d.scrollTop) {
							e.scrollIntoView({ block: "nearest" });
						} else {
							a.scrollIntoView({ block: "nearest" });
						}
					}
				});
				_.Fk([this.I0], () => {
					if (this.I0()) {
						this.Va.ta.set(true), this.Va.Cm.xc() || this.Va.Cm.Sa() || this.Va.Cm.reload(), this.A.UM().length !== 0 || this.A.Sa() || Dud(this);
					}
				});
				_.Fk([this.H], () => {
					Zsd(this.Ig, this.H());
				});
				_.Fk([this.I0], (a) => {
					if (this.I0()) {
						let b = (d) => {
							if (!this.Ga.nativeElement.contains(d.target)) {
								this.close(this.gz());
							}
						};
						let c = (d) => {
							if (d.relatedTarget && !this.Ga.nativeElement.contains(d.relatedTarget)) {
								this.close(this.gz());
							}
						};
						document.addEventListener("click", b);
						document.addEventListener("focusout", c);
						a(() => {
							document.removeEventListener("click", b);
							document.removeEventListener("focusout", c);
						});
					}
				});
			}
			a8(a) {
				this.selectedIndex.set(-1);
				this.Rja.set(false);
				if (a.endsWith(" ") && dtd(this.Ig, a)) {
					this.selectedIndex.set(0), this.YD();
				}
			}
			preventClose(a) {
				a.stopPropagation();
			}
			rE(a) {
				var b = this.aR();
				var c = this.stack();
				switch (a.key) {
					case "ArrowDown":
						if (b.length === 0) break;
						a.preventDefault();
						this.Rja.set(true);
						this.selectedIndex.update((d) => Math.min(b.length - 1, d + 1));
						break;
					case "ArrowUp":
						if (b.length === 0) break;
						a.preventDefault();
						this.Rja.set(true);
						this.selectedIndex.update((d) => Math.max(-1, d - 1));
						break;
					case "Enter":
						a.preventDefault();
						this.Jja();
						break;
					case "Backspace":
						this.Ig.query() === "" && c && c.length > 1 && Eud(this);
						break;
					case "Escape": this.close();
				}
			}
			Jja() {
				var a = this.Um();
				var b = this.Ig.query();
				if (a) {
					dtd(this.Ig, b) ? (this.selectedIndex.set(0), this.YD()) : a.source === null ? (b || a.Cwb) && Fud(this, b) : (a = this.aR(), b = this.selectedIndex(), a.length > 0 && (b === -1 ? esd(this, a[0]) : b < a.length && esd(this, a[b])));
				}
			}
			YD() {
				var a;
				if (!((a = this.Fka()) == null)) {
					a.nativeElement.focus();
				}
				this.Ig.open(3);
			}
			YFa() {
				this.Ig.open(3);
			}
			close(a = true) {
				this.Ig.close(a);
				if (a) {
					this.stack.update((c) => c && c.length > 0 ? [c[0]] : c);
				}
				var b;
				if (!((b = this.Fka()) == null)) {
					b.nativeElement.blur();
				}
			}
		};
		Gud.J = function(a) {
			return new (a || Gud)();
		};
		Gud.ka = _.u({
			type: Gud,
			da: [["ms-omnibar"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.Fka, Aud, 5)(b.Z8a, Bud, 5)(b.DDa, Cud, 5);
				}
				if (a & 2) {
					_.ki(3);
				}
			},
			Ua: 2,
			Ja: function(a, b) {
				if (a & 2) {
					_.P("inline-mode", !b.gz());
				}
			},
			inputs: { gz: [1, "isOverlay"] },
			ha: 4,
			ia: 1,
			la: [
				["omnibarContent", ""],
				["overlayContent", ""],
				["inputField", ""],
				["resultsList", ""],
				["resultItem", ""],
				[
					3,
					"ve",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					1,
					"overlay-backdrop",
					3,
					"ve",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					1,
					"overlay-backdrop",
					3,
					"click",
					"ve",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					"role",
					"dialog",
					"aria-modal",
					"true",
					"aria-label",
					"Command Palette",
					"cdkTrapFocus",
					"",
					"cdkTrapFocusAutoCapture",
					"",
					1,
					"overlay-content",
					3,
					"click"
				],
				[4, "ngTemplateOutlet"],
				[
					1,
					"input-wrapper",
					3,
					"mousedown",
					"click"
				],
				[
					"aria-live",
					"assertive",
					"role",
					"alert",
					1,
					"feedback-overlay"
				],
				[1, "chips-wrapper"],
				[1, "input-content"],
				[
					1,
					"input-icon",
					3,
					"iconName"
				],
				[
					1,
					"ve-container",
					3,
					"ve",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					"type",
					"text",
					"role",
					"combobox",
					"aria-haspopup",
					"listbox",
					"aria-autocomplete",
					"list",
					"aria-controls",
					"omnibar-results",
					3,
					"ngModelChange",
					"keydown",
					"focus",
					"placeholder",
					"aria-label",
					"ngModel",
					"aria-expanded",
					"ve",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					"aria-hidden",
					"true",
					1,
					"shortcut",
					"input-shortcut-hint"
				],
				[
					"aria-label",
					"Submit",
					1,
					"action-button",
					3,
					"ve",
					"veClick",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[1, "menu-overlay"],
				[1, "feedback-icon"],
				[3, "iconName"],
				[1, "feedback-message"],
				[
					1,
					"chip",
					3,
					"ve",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					1,
					"chip-icon",
					3,
					"iconName"
				],
				[1, "chip-label"],
				[
					"aria-label",
					"Submit",
					1,
					"action-button",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					1,
					"menu-overlay",
					3,
					"click"
				],
				[1, "empty-state"],
				[
					"role",
					"listbox",
					"id",
					"omnibar-results",
					"tabindex",
					"-1",
					1,
					"results-list"
				],
				[
					"role",
					"status",
					"aria-live",
					"polite",
					1,
					"empty-state"
				],
				["role", "presentation"],
				[
					"role",
					"option",
					1,
					"result-item",
					3,
					"selected",
					"id",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					"role",
					"presentation",
					1,
					"category-header"
				],
				[
					"role",
					"option",
					"aria-hidden",
					"true",
					1,
					"result-item",
					"skeleton-item"
				],
				[
					"role",
					"option",
					"aria-hidden",
					"true",
					1,
					"result-item",
					"skeleton-item",
					3,
					"click"
				],
				[
					1,
					"icon-container",
					"skeleton"
				],
				[1, "item-content"],
				[
					1,
					"item-label",
					"skeleton"
				],
				[
					1,
					"item-description",
					"skeleton"
				],
				[
					"role",
					"option",
					1,
					"result-item",
					3,
					"click",
					"id",
					"ve",
					"veClick",
					"veImpression",
					"veMetadata",
					"veMutable"
				],
				[
					4,
					"ngTemplateOutlet",
					"ngTemplateOutletContext"
				],
				[1, "icon-container"],
				[1, "item-label"],
				[1, "item-description"],
				[1, "shortcut"],
				[
					1,
					"menu-chevron",
					3,
					"iconName"
				],
				[
					1,
					"item-icon",
					3,
					"iconName"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, Ard, 1, 1)(1, Crd, 3, 7, "div", 5), _.z(2, usd, 4, 4, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					_.C(b.gz() ? 0 : 1);
				}
			},
			dependencies: [
				_.TA,
				_.JA,
				_.tz,
				_.nz,
				_.JD,
				_.Wn,
				_.oD,
				_.vD,
				_.dz,
				_.OD,
				_.ND,
				_.Cz,
				_.Bz,
				_.iz
			],
			styles: ["[_nghost-%COMP%]{display:block;width:100%;position:relative}[_nghost-%COMP%]:not(.inline-mode){max-width:600px}.input-wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;gap:8px;padding:6px 12px;background:var(--color-v3-surface-container);cursor:text;border-radius:12px;border:1px solid var(--color-v3-outline);box-shadow:var(--v3-shadow-lg);min-height:56px;-moz-box-sizing:border-box;box-sizing:border-box;-webkit-transition:all .2s ease;transition:all .2s ease;position:relative;overflow:hidden}@media screen and (max-width:600px){.input-wrapper[_ngcontent-%COMP%]{-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:stretch;-webkit-align-items:stretch;-moz-box-align:stretch;-ms-flex-align:stretch;align-items:stretch;gap:4px}}.input-wrapper[_ngcontent-%COMP%]:focus-within{border-color:var(--color-v3-outline-accent);background:var(--color-v3-surface)}.input-wrapper[_ngcontent-%COMP%]:focus-within   .input-icon[_ngcontent-%COMP%]{color:var(--color-v3-text);opacity:1}.input-wrapper[_ngcontent-%COMP%]   .input-icon[_ngcontent-%COMP%]{color:var(--color-v3-text-var);font-size:20px;margin-right:4px;opacity:.7;-webkit-transition:all .2s ease;transition:all .2s ease;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.input-wrapper[_ngcontent-%COMP%]   input[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;min-width:120px;border:none;background:transparent;height:44px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;padding:0;color:var(--color-v3-text);outline:none}.input-wrapper[_ngcontent-%COMP%]   input[_ngcontent-%COMP%]::-webkit-input-placeholder{color:var(--color-v3-text-var)}.input-wrapper[_ngcontent-%COMP%]   input[_ngcontent-%COMP%]::-moz-placeholder{color:var(--color-v3-text-var)}.input-wrapper[_ngcontent-%COMP%]   input[_ngcontent-%COMP%]:-ms-input-placeholder{color:var(--color-v3-text-var)}.input-wrapper[_ngcontent-%COMP%]   input[_ngcontent-%COMP%]::-ms-input-placeholder{color:var(--color-v3-text-var)}.input-wrapper[_ngcontent-%COMP%]   input[_ngcontent-%COMP%]::placeholder{color:var(--color-v3-text-var)}.input-wrapper[_ngcontent-%COMP%]   .input-shortcut-hint[_ngcontent-%COMP%]{display:none;margin-left:4px}@media (min-width:600px){.input-wrapper[_ngcontent-%COMP%]   .input-shortcut-hint[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}}.input-wrapper[_ngcontent-%COMP%]   .action-button[_ngcontent-%COMP%]{background:transparent;border:none;color:var(--color-v3-text-disable);cursor:pointer;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:4px;border-radius:8px;-webkit-transition:all .2s ease;transition:all .2s ease;margin-left:4px}.input-wrapper[_ngcontent-%COMP%]   .action-button[_ngcontent-%COMP%]:hover{background:var(--color-v3-hover);color:var(--color-v3-text)}.input-wrapper[_ngcontent-%COMP%]   .action-button[_ngcontent-%COMP%]   span[_ngcontent-%COMP%]{font-size:20px}.inline-mode[_nghost-%COMP%]   .input-wrapper[_ngcontent-%COMP%]{padding:12px 16px}.inline-mode[_nghost-%COMP%]   input[_ngcontent-%COMP%]{font-size:16px}.chips-wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;gap:6px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}.ve-container[_ngcontent-%COMP%]{display:none}.input-content[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;width:100%;gap:8px}.shortcut[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:4px}.shortcut[_ngcontent-%COMP%]   kbd[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var);background-color:var(--color-v3-surface-container-high);border:1px solid var(--color-v3-outline-variant);border-radius:4px;padding:4px 8px;min-width:20px;text-align:center}.feedback-overlay[_ngcontent-%COMP%]{position:absolute;inset:0;z-index:20;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;background:var(--color-v3-surface-container);border-radius:12px;gap:8px}@-webkit-keyframes _ngcontent-%COMP%_fadeIn{0%{opacity:0}to{opacity:1}}.feedback-overlay[_ngcontent-%COMP%]{-webkit-animation:_ngcontent-%COMP%_fadeIn .2s ease;animation:_ngcontent-%COMP%_fadeIn .2s ease}.feedback-overlay[_ngcontent-%COMP%]   .feedback-icon[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center}.feedback-overlay[_ngcontent-%COMP%]   .feedback-icon.success[_ngcontent-%COMP%]{width:20px;height:20px;border-radius:9999px;background:var(--color-v3-accent-light-4);background:rgba(60,219,133,.1);color:var(--color-v3-accent-4)}.feedback-overlay[_ngcontent-%COMP%]   .feedback-icon.success[_ngcontent-%COMP%]   span[_ngcontent-%COMP%]{font-size:14px}.feedback-overlay[_ngcontent-%COMP%]   .feedback-icon.loading[_ngcontent-%COMP%]{color:var(--color-v3-outline-accent);-webkit-animation:_ngcontent-%COMP%_pulse 1.5s infinite;animation:_ngcontent-%COMP%_pulse 1.5s infinite}.feedback-overlay[_ngcontent-%COMP%]   .feedback-message[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text)}@-webkit-keyframes _ngcontent-%COMP%_pulse{0%,to{opacity:.6}50%{opacity:1}}@keyframes _ngcontent-%COMP%_pulse{0%,to{opacity:.6}50%{opacity:1}}.menu-overlay[_ngcontent-%COMP%]{position:absolute;top:calc(100% + 8px);left:0;right:0;background:var(--color-v3-surface-container);border:1px solid var(--color-v3-outline-var);border-radius:12px;box-shadow:var(--v3-shadow-lg);overflow:hidden;z-index:1100;-webkit-animation:_ngcontent-%COMP%_slideDown .2s ease-out;animation:_ngcontent-%COMP%_slideDown .2s ease-out}@-webkit-keyframes _ngcontent-%COMP%_slideDown{0%{opacity:0;-webkit-transform:translateY(-10px);transform:translateY(-10px)}to{opacity:1;-webkit-transform:translateY(0);transform:translateY(0)}}@keyframes _ngcontent-%COMP%_slideDown{0%{opacity:0;-webkit-transform:translateY(-10px);transform:translateY(-10px)}to{opacity:1;-webkit-transform:translateY(0);transform:translateY(0)}}@keyframes _ngcontent-%COMP%_fadeIn{0%{opacity:0}to{opacity:1}}.chip[_ngcontent-%COMP%]{background:var(--color-v3-surface-container-high);color:var(--color-v3-text);border:1px solid var(--color-v3-outline);border-radius:4px;padding:0 8px;height:28px;font-weight:500;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:6px;white-space:nowrap;-webkit-animation:_ngcontent-%COMP%_scaleIn .2s ease-out;animation:_ngcontent-%COMP%_scaleIn .2s ease-out;pointer-events:none}.chip[_ngcontent-%COMP%]   .chip-icon[_ngcontent-%COMP%]{font-size:18px;width:18px;height:18px}@-webkit-keyframes _ngcontent-%COMP%_scaleIn{0%{opacity:0;-webkit-transform:scale(.95);transform:scale(.95)}to{opacity:1;-webkit-transform:scale(1);transform:scale(1)}}@keyframes _ngcontent-%COMP%_scaleIn{0%{opacity:0;-webkit-transform:scale(.95);transform:scale(.95)}to{opacity:1;-webkit-transform:scale(1);transform:scale(1)}}.results-list[_ngcontent-%COMP%]{max-height:50vh;overflow-y:auto;padding:6px;position:relative;list-style:none;margin:0}.skeleton[_ngcontent-%COMP%]{-webkit-animation-duration:1.7s;animation-duration:1.7s;-webkit-animation-fill-mode:forwards;animation-fill-mode:forwards;-webkit-animation-iteration-count:infinite;animation-iteration-count:infinite;-webkit-animation-timing-function:linear;animation-timing-function:linear;-webkit-animation-name:_ngcontent-%COMP%_moving-gradient;animation-name:_ngcontent-%COMP%_moving-gradient;background:-webkit-linear-gradient(160deg,var(--color-loading-background) 40%,var(--color-loading-background-contrast) 50%,var(--color-loading-background) 60%);background:linear-gradient(290deg,var(--color-loading-background) 40%,var(--color-loading-background-contrast) 50%,var(--color-loading-background) 60%);background-size:800px;border-radius:4px;display:block;color:transparent;-webkit-user-select:none;-moz-user-select:none;-ms-user-select:none;user-select:none}@-webkit-keyframes _ngcontent-%COMP%_moving-gradient{0%{background-position:-400px 0}to{background-position:400px 0}}@keyframes _ngcontent-%COMP%_moving-gradient{0%{background-position:-400px 0}to{background-position:400px 0}}.skeleton-item[_ngcontent-%COMP%]{cursor:default;pointer-events:none}.skeleton-item[_ngcontent-%COMP%]:hover{background:transparent}.skeleton-item[_ngcontent-%COMP%]   .icon-container.skeleton[_ngcontent-%COMP%]{border:none}.skeleton-item[_ngcontent-%COMP%]   .item-label.skeleton[_ngcontent-%COMP%]{width:50%;height:16px;margin-bottom:6px}.skeleton-item[_ngcontent-%COMP%]   .item-description.skeleton[_ngcontent-%COMP%]{width:50%;height:12px}.category-header[_ngcontent-%COMP%]{cursor:default;padding:12px 12px 4px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-var);margin-top:4px}.category-header[_ngcontent-%COMP%]:first-child{margin-top:0}.result-item[_ngcontent-%COMP%]{padding:8px 12px;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:12px;cursor:pointer;color:var(--color-v3-text);border-radius:8px;margin-bottom:2px;-webkit-transition:background-color .1s ease;transition:background-color .1s ease}.result-item[_ngcontent-%COMP%]:hover{background:var(--color-v3-hover)}.result-item[_ngcontent-%COMP%]:hover   .icon-container[_ngcontent-%COMP%]{background:var(--color-v3-button-container-high);border-color:var(--color-v3-outline)}.result-item[_ngcontent-%COMP%]:hover   .item-icon[_ngcontent-%COMP%]{color:var(--color-v3-text)}.result-item.selected[_ngcontent-%COMP%]{background:var(--color-v3-button-container-high)}.result-item.selected[_ngcontent-%COMP%]   .icon-container[_ngcontent-%COMP%]{background:var(--color-v3-button-container-highest);border-color:var(--color-v3-outline)}.result-item[_ngcontent-%COMP%]   .icon-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;width:32px;height:32px;border-radius:8px;border:1px solid var(--color-v3-outline-var);background:var(--color-v3-surface-container-high);-webkit-transition:all .2s ease;transition:all .2s ease;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.result-item[_ngcontent-%COMP%]   .item-icon[_ngcontent-%COMP%]{color:var(--color-v3-text-var)}.result-item[_ngcontent-%COMP%]   .item-content[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1;min-width:0;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column}.result-item[_ngcontent-%COMP%]   .item-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px}.result-item[_ngcontent-%COMP%]   .item-description[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var);display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin:0}.result-item[_ngcontent-%COMP%]   .shortcut[_ngcontent-%COMP%]{margin-left:8px}.result-item[_ngcontent-%COMP%]   .menu-chevron[_ngcontent-%COMP%]{color:var(--color-v3-text-var);font-size:18px;width:18px;height:18px}.empty-state[_ngcontent-%COMP%]{padding:16px;text-align:center;color:var(--color-v3-text-var);font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}.overlay-backdrop[_ngcontent-%COMP%]{position:fixed;inset:0;-webkit-backdrop-filter:blur(2px);backdrop-filter:blur(2px);background-color:var(--color-v3-overlay-background);z-index:1100;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-animation:_ngcontent-%COMP%_fadeIn .2s ease-in-out;animation:_ngcontent-%COMP%_fadeIn .2s ease-in-out}@media screen and (max-width:600px){.overlay-backdrop[_ngcontent-%COMP%]{-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;padding:12px}}.overlay-content[_ngcontent-%COMP%]{width:100%;max-width:600px;border-radius:8px;position:relative;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;min-height:calc(50vh + 68px)}.overlay-content[_ngcontent-%COMP%]   .menu-overlay[_ngcontent-%COMP%]{position:static;margin-top:8px}"]
		});
		var Hud = new _.$y("45776055", true);
		var Jud = () => [Gud];
		var Dsd = [
			"500",
			"404",
			"520"
		];
		var Lud = function(a) {
			return _.x(function* () {
				yield Usd(a.A);
				yield Kud(a);
			});
		};
		var Kud = function(a) {
			return _.x(function* () {
				var b = a.route.snapshot.Op.get("importid");
				var c = a.route.snapshot.Op.get("sig");
				var d = a.route.snapshot.Op.get("applet_origin");
				var e = a.route.snapshot.Op.get("project");
				if (b && c) {
					var f;
					if ((f = a.A.F()) != null && f.Ika) {
						a.HCa.set(true);
						try {
							let g;
							g = (d == null ? undefined : d.toLowerCase()) === "stitch" ? yield zud(a.R, b, c) : yield yud(a.R, b, c, e != null ? e : undefined);
							yield a.ea.navigate([
								"/",
								"apps",
								g
							]);
						} catch (g) {
							if (_.pn(g)) {
								a.I.error("Permission denied. You may not have access to import this app.");
							} else {
								a.I.error("Unable to import app. Please try again later.");
							}
						} finally {
							a.HCa.set(false);
						}
					}
				}
			});
		};
		_.Rr = class {
			constructor() {
				this.S = _.Dk;
				this.E4a = _.Ni("mainContent");
				this.U = _.m(_.Op);
				this.aCb = this.U.getFlag(Hud);
				this.aa = _.m(_.BF);
				this.A = _.m(_.yG);
				this.dialog = _.m(_.rC);
				this.H = _.m(_.pG);
				this.fa = _.m(_.rF);
				this.Qc = _.m(_.BM);
				this.ma = _.m(_.HG);
				this.window = _.m(_.Sn);
				this.F = _.m(_.x3);
				this.Ig = _.m(_.C3);
				this.X = _.m(_.nK);
				this.R = _.m(e4);
				this.ea = _.m(_.Cl);
				this.route = _.m(_.ll);
				this.I = _.m(_.iC);
				this.bDa = _.M(!this.window.navigator.onLine);
				this.HCa = _.M(false);
				this.ma.A.set(true);
				_.Fk([this.H.url, this.A.F], () => {
					var a = this;
					return _.x(function* () {
						var b = a.H.url();
						var c = !!a.A.F();
						if (Esd(b) && c) {
							yield Lud(a);
						}
					});
				});
				_.OBb(this.X);
			}
			ib() {
				this.init();
			}
			Ba() {
				var a;
				if (!((a = this.F) == null)) {
					a.unregister("open-omnibar-overlay");
				}
			}
			init() {
				_.iob(this.fa);
				Osd(this.aa);
				this.F.init();
				this.F.register({
					id: "open-omnibar-overlay",
					keys: ["ctrl", "/"],
					St: ["meta", "/"],
					Jpa: true,
					action: () => {
						this.Ig.open(1);
					},
					label: "Open command palette",
					description: "Open the command palette anywhere",
					variant: "command"
				});
			}
		};
		_.Rr.J = function(a) {
			return new (a || _.Rr)();
		};
		_.Rr.ka = _.u({
			type: _.Rr,
			da: [["ms-app"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.E4a, Iud, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			Ja: function(a, b) {
				if (a & 1) {
					_.J("offline", function() {
						return b.bDa.set(true);
					}, _.Te)("online", function() {
						return b.bDa.set(false);
					}, _.Te);
				}
			},
			ha: 20,
			ia: 6,
			la: [
				["mainContent", ""],
				[
					"rel",
					"preconnect",
					"href",
					Kod`https://fonts.googleapis.com`
				],
				[
					"rel",
					"preconnect",
					"href",
					Kod`https://fonts.gstatic.com`,
					"crossorigin",
					""
				],
				[1, "banner-and-app-container"],
				[
					1,
					"skip-content",
					3,
					"click"
				],
				[1, "makersuite-layout"],
				[1, "offline-banner-wrapper"],
				[1, "sidebar-overlay"],
				[
					"role",
					"main",
					1,
					"layout-wrapper"
				],
				[
					"tabindex",
					"-1",
					1,
					"layout-main"
				],
				[1, "router-outlet-wrapper"],
				[1, "import-overlay"],
				[3, "isOverlay"],
				[1, "offline-banner"],
				[3, "iconName"],
				[
					1,
					"sidebar-overlay",
					3,
					"click"
				],
				[1, "import-loading-container"],
				["diameter", "48"],
				[1, "import-loading-text"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.I(0, "link", 1)(1, "link", 2), _.F(2, "div", 3)(3, "button", 4), _.J("click", function() {
						var c;
						if (!((c = b.E4a()) == null)) {
							c.nativeElement.focus();
						}
					}), _.R(4, "Skip to main content"), _.H(), _.I(5, "ms-banner"), _.ph(6, vsd, 1, 1), _.qh(7, 6, Jud), _.F(9, "div", 5), _.B(10, wsd, 5, 2, "div", 6), _.B(11, xsd, 1, 1, "div", 7), _.B(12, ysd, 1, 1, "div", 7), _.B(13, Bsd, 2, 1), _.F(14, "div", 8)(15, "div", 9, 0)(17, "span", 10), _.I(18, "router-outlet"), _.H()()()(), _.B(19, Csd, 5, 0, "div", 11), _.H();
				}
				if (a & 2) {
					_.y(7), _.Sma(true), _.y(3), _.C(b.bDa() ? 10 : -1), _.y(), _.C(b.Qc.isNavbarExpanded() ? 11 : -1), _.y(), _.C(b.Qc.OB() ? 12 : -1), _.y(), _.C(b.Qc.BF() ? 13 : -1), _.y(6), _.C(b.HCa() ? 19 : -1);
				}
			},
			dependencies: [
				ltd,
				_.dz,
				_.zC,
				_.yC,
				Ytd,
				wud,
				_.Iz
			],
			styles: ["[_nghost-%COMP%]{background:var(--color-surface);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:1;-webkit-flex-grow:1;-moz-box-flex:1;-ms-flex-positive:1;flex-grow:1;height:0;width:100%}.skip-content[_ngcontent-%COMP%]{left:-500px;position:absolute}.skip-content[_ngcontent-%COMP%]:focus{left:0;-webkit-transform:translateX(0);transform:translateX(0);background-color:var(--color-surface);z-index:3}header[_ngcontent-%COMP%]{-webkit-box-flex:0;-webkit-flex:0 0;-moz-box-flex:0;-ms-flex:0 0;flex:0 0;z-index:3}.banner-and-app-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;height:100%;overflow:hidden;background:var(--color-v3-surface);-webkit-backdrop-filter:blur(200px);backdrop-filter:blur(200px);width:100%}.sidebar-overlay[_ngcontent-%COMP%]{display:none}@media screen and (max-width:960px){.sidebar-overlay[_ngcontent-%COMP%]{background:var(--color-overlay-background);display:block;height:100%;position:absolute;width:100%;z-index:4}}.router-outlet-wrapper[_ngcontent-%COMP%]{display:block;height:100%}.offline-banner-wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;left:0;position:absolute;right:0;top:76px;z-index:5}.offline-banner-wrapper[_ngcontent-%COMP%]   .offline-banner[_ngcontent-%COMP%]{border-radius:6px;border:1px solid var(--color-v3-outline);background:var(--color-v3-surface-container);color:var(--color-v3-text);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;margin-inline:8px;padding:12px;box-shadow:var(--v3-shadow-lg)}.offline-banner-wrapper[_ngcontent-%COMP%]   .offline-banner[_ngcontent-%COMP%]   span.material-symbols-outlined[_ngcontent-%COMP%]{color:var(--color-v3-accent-3);margin-right:12px}.import-overlay[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;background:rgba(0,0,0,.5);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;inset:0;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;position:fixed;z-index:1002}.import-loading-container[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;background:var(--color-v3-surface-container);border-radius:16px;box-shadow:var(--v3-shadow-lg);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:12px;padding:20px}.import-loading-text[_ngcontent-%COMP%]{color:var(--color-v3-text);font-size:16px;font-weight:500}"],
			data: { animation: [_.umd] }
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

