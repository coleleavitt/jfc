"use strict";
this.default_MakerSuite = this.default_MakerSuite || {};
(function(_) {
	try {
		var PXb;
		_.nO = function(a, b = _.Bf, c = _.Vla) {
			var d = _.Cf(a, b);
			return _.Wla(() => d, c);
		};
		_.LXb = function(a, b) {
			for (let c in a) if (!(c in b) || a[c] !== b[c]) return false;
			for (let c in b) if (!(c in a)) return false;
			return true;
		};
		MXb = function(a) {
			if (a & 1) {
				_.Fh(0, "div", 2);
			}
		};
		_.N9a.prototype.Du = _.ca(108, function() {
			return _.In(this, 3);
		});
		_.zab.prototype.Du = _.ca(107, function() {
			return _.In(this, 2);
		});
		_.oK.prototype.Du = _.ca(106, function() {
			return _.In(this, 2);
		});
		_.UBb.prototype.Du = _.ca(105, function() {
			return _.In(this, 8);
		});
		_.oO = class {
			get aL() {
				var a;
				var b;
				return (b = (a = this.H) == null ? undefined : a.instance) != null ? b : null;
			}
			constructor(a) {
				this.I = a;
				this.iFa = null;
				this.A = new Map();
			}
			R(a) {
				return a.ngComponentOutletNgModule !== undefined;
			}
			X(a) {
				return a.ngComponentOutlet !== undefined || a.ngComponentOutletContent !== undefined || a.ngComponentOutletInjector !== undefined || a.ngComponentOutletEnvironmentInjector !== undefined || this.R(a);
			}
			Wb(a) {
				if (this.X(a) && (this.I.clear(), this.A.clear(), this.H = undefined, this.iFa)) {
					let c = this.cLb || this.I.I;
					if (this.R(a)) {
						var b;
						if (!((b = this.F) == null)) {
							b.destroy();
						}
						if (this.P5a) {
							a = this.P5a, b = c.get(_.wu).Pa, this.F = new _.oSa(a, b != null ? b : null);
						} else {
							this.F = undefined;
						}
					}
					this.H = _.Fu(this.I, this.iFa, {
						Pa: c,
						nFa: this.F,
						RI: this.aLb,
						Oy: this.bLb
					});
				}
			}
			ws() {
				if (this.H) {
					if (this.jFa) for (let a of Object.keys(this.jFa)) this.A.set(a, true);
					this.U(this.H);
				}
			}
			Ba() {
				var a;
				if (!((a = this.F) == null)) {
					a.destroy();
				}
			}
			U(a) {
				for (let [b, c] of this.A) {
					let d = b;
					if (c) {
						a.zk(d, this.jFa[d]), this.A.set(d, false);
					} else {
						a.zk(d, undefined), this.A.delete(d);
					}
				}
			}
		};
		_.oO.J = function(a) {
			return new (a || _.oO)(_.Dg(_.$h));
		};
		_.oO.Oa = _.We({
			type: _.oO,
			da: [[
				"",
				"ngComponentOutlet",
				""
			]],
			inputs: {
				iFa: "ngComponentOutlet",
				jFa: "ngComponentOutletInputs",
				cLb: "ngComponentOutletInjector",
				bLb: "ngComponentOutletEnvironmentInjector",
				aLb: "ngComponentOutletContent",
				P5a: "ngComponentOutletNgModule"
			},
			Cc: ["ngComponentOutlet"],
			features: [_.su]
		});
		NXb = class {
			constructor(a, b) {
				this.F = a;
				this.H = b;
				this.A = false;
			}
			create() {
				this.A = true;
				this.F.wo(this.H);
			}
			destroy() {
				this.A = false;
				this.F.clear();
			}
		};
		_.pO = class {
			constructor() {
				this.H = [];
				this.R = false;
				this.I = this.F = 0;
				this.A = false;
			}
			set oFa(a) {
				this.fa = a;
				if (this.F === 0) {
					this.U(true);
				}
			}
			X() {
				return this.F++;
			}
			aa(a) {
				this.H.push(a);
			}
			ea(a) {
				a = a === this.fa;
				if (!this.A) {
					this.A = a;
				}
				this.I++;
				if (this.I === this.F) {
					this.U(!this.A), this.I = 0, this.A = false;
				}
				return a;
			}
			U(a) {
				if (this.H.length > 0 && a !== this.R) {
					this.R = a;
					for (let d of this.H) {
						var b = d;
						var c = a;
						if (c && !b.A) {
							b.create();
						} else {
							if (!c && b.A) {
								b.destroy();
							}
						}
					}
				}
			}
		};
		_.pO.J = function(a) {
			return new (a || _.pO)();
		};
		_.pO.Oa = _.We({
			type: _.pO,
			da: [[
				"",
				"ngSwitch",
				""
			]],
			inputs: { oFa: "ngSwitch" }
		});
		_.qO = class {
			constructor(a, b, c) {
				this.oFa = c;
				c.X();
				this.A = new NXb(a, b);
			}
			ws() {
				var a = this.A;
				var b = this.oFa.ea(this.hLb);
				if (b && !a.A) {
					a.create();
				} else {
					if (!b && a.A) {
						a.destroy();
					}
				}
			}
		};
		_.qO.J = function(a) {
			return new (a || _.qO)(_.Dg(_.$h), _.Dg(_.Zh), _.Dg(_.pO, 9));
		};
		_.qO.Oa = _.We({
			type: _.qO,
			da: [[
				"",
				"ngSwitchCase",
				""
			]],
			inputs: { hLb: "ngSwitchCase" }
		});
		_.rO = class {
			constructor(a, b, c) {
				c.aa(new NXb(a, b));
			}
		};
		_.rO.J = function(a) {
			return new (a || _.rO)(_.Dg(_.$h), _.Dg(_.Zh), _.Dg(_.pO, 9));
		};
		_.rO.Oa = _.We({
			type: _.rO,
			da: [[
				"",
				"ngSwitchDefault",
				""
			]]
		});
		_.OXb = class extends _.bib {
			constructor(a) {
				super();
				this.jh = a;
			}
			connect() {
				return _.Hf(this.jh) ? this.jh : _.mf(this.jh);
			}
			disconnect() {}
		};
		PXb = new _.he("MAT_PROGRESS_BAR_DEFAULT_OPTIONS");
		new _.he("mat-progress-bar-location", {
			wa: "root",
			factory: () => {
				var a = _.m(_.Xk);
				var b = a ? a.location : null;
				return { wAa: () => b ? b.pathname + b.search : "" };
			}
		});
		_.sO = class {
			constructor() {
				this.Ma = _.m(_.Jf);
				this.qb = _.m(_.th);
				this.wb = _.m(_.Hu);
				this.R = _.m(_.cm);
				this.A = "primary";
				this.F = this.ce = 0;
				this.Ava = new _.pm();
				this.H = "determinate";
				this.U = (c) => {
					if (this.Ava.observers.length !== 0 && c.target && c.target.classList.contains("mdc-linear-progress__primary-bar")) {
						this.mode !== "determinate" && this.mode !== "buffer" || this.qb.run(() => this.Ava.next({ value: this.value }));
					}
				};
				var a = _.uwa();
				var b = _.m(PXb, { optional: true });
				this.oSa = a === "di-disabled";
				if (a === "reduced-motion") {
					this.Ma.nativeElement.classList.add("mat-progress-bar-reduced-motion");
				}
				if (b) {
					b.color && (this.color = this.A = b.color), this.mode = b.mode || this.mode;
				}
			}
			get color() {
				return this.hp || this.A;
			}
			set color(a) {
				this.hp = a;
			}
			get value() {
				return this.ce;
			}
			set value(a) {
				this.ce = Math.max(0, Math.min(100, a || 0));
				this.wb.lb();
			}
			get qwa() {
				return this.F || 0;
			}
			set qwa(a) {
				this.F = Math.max(0, Math.min(100, a || 0));
				this.wb.lb();
			}
			get mode() {
				return this.H;
			}
			set mode(a) {
				this.H = a;
				this.wb.lb();
			}
			Rb() {
				this.qb.runOutsideAngular(() => {
					this.I = this.R.listen(this.Ma.nativeElement, "transitionend", this.U);
				});
			}
			Ba() {
				var a;
				if (!((a = this.I) == null)) {
					a();
				}
			}
			qub() {
				return `scaleX(${this.sua() ? 1 : this.value / 100})`;
			}
			lub() {
				return `${this.mode === "buffer" ? this.qwa : 100}%`;
			}
			sua() {
				return this.mode === "indeterminate" || this.mode === "query";
			}
		};
		_.sO.J = function(a) {
			return new (a || _.sO)();
		};
		_.sO.ka = _.u({
			type: _.sO,
			da: [["mat-progress-bar"]],
			eb: [
				"role",
				"progressbar",
				"aria-valuemin",
				"0",
				"aria-valuemax",
				"100",
				"tabindex",
				"-1",
				1,
				"mat-mdc-progress-bar",
				"mdc-linear-progress"
			],
			Ua: 10,
			Ja: function(a, b) {
				if (a & 2) {
					_.wh("aria-valuenow", b.sua() ? null : b.value)("mode", b.mode), _.qi("mat-" + b.color), _.P("_mat-animation-noopable", b.oSa)("mdc-linear-progress--animation-ready", !b.oSa)("mdc-linear-progress--indeterminate", b.sua());
				}
			},
			inputs: {
				color: "color",
				value: [
					2,
					"value",
					"value",
					_.bj
				],
				qwa: [
					2,
					"bufferValue",
					"bufferValue",
					_.bj
				],
				mode: "mode"
			},
			outputs: { Ava: "animationEnd" },
			Cc: ["matProgressBar"],
			ha: 7,
			ia: 5,
			la: [
				[
					"aria-hidden",
					"true",
					1,
					"mdc-linear-progress__buffer"
				],
				[1, "mdc-linear-progress__buffer-bar"],
				[1, "mdc-linear-progress__buffer-dots"],
				[
					"aria-hidden",
					"true",
					1,
					"mdc-linear-progress__bar",
					"mdc-linear-progress__primary-bar"
				],
				[1, "mdc-linear-progress__bar-inner"],
				[
					"aria-hidden",
					"true",
					1,
					"mdc-linear-progress__bar",
					"mdc-linear-progress__secondary-bar"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Dh(0, "div", 0), _.Fh(1, "div", 1), _.B(2, MXb, 1, 0, "div", 2), _.Eh(), _.Dh(3, "div", 3), _.Fh(4, "span", 4), _.Eh(), _.Dh(5, "div", 5), _.Fh(6, "span", 4), _.Eh();
				}
				if (a & 2) {
					_.y(), _.pi("flex-basis", b.lub()), _.y(), _.C(b.mode === "buffer" ? 2 : -1), _.y(), _.pi("transform", b.qub());
				}
			},
			styles: [".mat-mdc-progress-bar{--mat-progress-bar-animation-multiplier: 1;display:block;text-align:start}.mat-mdc-progress-bar[mode=query]{transform:scaleX(-1)}.mat-mdc-progress-bar._mat-animation-noopable .mdc-linear-progress__buffer-dots,.mat-mdc-progress-bar._mat-animation-noopable .mdc-linear-progress__primary-bar,.mat-mdc-progress-bar._mat-animation-noopable .mdc-linear-progress__secondary-bar,.mat-mdc-progress-bar._mat-animation-noopable .mdc-linear-progress__bar-inner.mdc-linear-progress__bar-inner{animation:none}.mat-mdc-progress-bar._mat-animation-noopable .mdc-linear-progress__primary-bar,.mat-mdc-progress-bar._mat-animation-noopable .mdc-linear-progress__buffer-bar{transition:transform 1ms}.mat-progress-bar-reduced-motion{--mat-progress-bar-animation-multiplier: 2}.mdc-linear-progress{position:relative;width:100%;transform:translateZ(0);outline:1px solid rgba(0,0,0,0);overflow-x:hidden;transition:opacity 250ms 0ms cubic-bezier(0.4, 0, 0.6, 1);height:max(var(--mat-progress-bar-track-height, 4px),var(--mat-progress-bar-active-indicator-height, 4px))}@media(forced-colors: active){.mdc-linear-progress{outline-color:CanvasText}}.mdc-linear-progress__bar{position:absolute;top:0;bottom:0;margin:auto 0;width:100%;animation:none;transform-origin:top left;transition:transform 250ms 0ms cubic-bezier(0.4, 0, 0.6, 1);height:var(--mat-progress-bar-active-indicator-height, 4px)}.mdc-linear-progress--indeterminate .mdc-linear-progress__bar{transition:none}[dir=rtl] .mdc-linear-progress__bar{right:0;transform-origin:center right}.mdc-linear-progress__bar-inner{display:inline-block;position:absolute;width:100%;animation:none;border-top-style:solid;border-color:var(--mat-progress-bar-active-indicator-color, var(--mat-sys-primary));border-top-width:var(--mat-progress-bar-active-indicator-height, 4px)}.mdc-linear-progress__buffer{display:flex;position:absolute;top:0;bottom:0;margin:auto 0;width:100%;overflow:hidden;height:var(--mat-progress-bar-track-height, 4px);border-radius:var(--mat-progress-bar-track-shape, var(--mat-sys-corner-none))}.mdc-linear-progress__buffer-dots{background-image:radial-gradient(circle, var(--mat-progress-bar-track-color, var(--mat-sys-surface-variant)) calc(var(--mat-progress-bar-track-height, 4px) / 2), transparent 0);background-repeat:repeat-x;background-size:calc(calc(var(--mat-progress-bar-track-height, 4px) / 2)*5);background-position:left;flex:auto;transform:rotate(180deg);animation:mdc-linear-progress-buffering calc(250ms*var(--mat-progress-bar-animation-multiplier)) infinite linear}@media(forced-colors: active){.mdc-linear-progress__buffer-dots{background-color:ButtonBorder}}[dir=rtl] .mdc-linear-progress__buffer-dots{animation:mdc-linear-progress-buffering-reverse calc(250ms*var(--mat-progress-bar-animation-multiplier)) infinite linear;transform:rotate(0)}.mdc-linear-progress__buffer-bar{flex:0 1 100%;transition:flex-basis 250ms 0ms cubic-bezier(0.4, 0, 0.6, 1);background-color:var(--mat-progress-bar-track-color, var(--mat-sys-surface-variant))}.mdc-linear-progress__primary-bar{transform:scaleX(0)}.mdc-linear-progress--indeterminate .mdc-linear-progress__primary-bar{left:-145.166611%}.mdc-linear-progress--indeterminate.mdc-linear-progress--animation-ready .mdc-linear-progress__primary-bar{animation:mdc-linear-progress-primary-indeterminate-translate calc(2s*var(--mat-progress-bar-animation-multiplier)) infinite linear}.mdc-linear-progress--indeterminate.mdc-linear-progress--animation-ready .mdc-linear-progress__primary-bar>.mdc-linear-progress__bar-inner{animation:mdc-linear-progress-primary-indeterminate-scale calc(2s*var(--mat-progress-bar-animation-multiplier)) infinite linear}[dir=rtl] .mdc-linear-progress.mdc-linear-progress--animation-ready .mdc-linear-progress__primary-bar{animation-name:mdc-linear-progress-primary-indeterminate-translate-reverse}[dir=rtl] .mdc-linear-progress.mdc-linear-progress--indeterminate .mdc-linear-progress__primary-bar{right:-145.166611%;left:auto}.mdc-linear-progress__secondary-bar{display:none}.mdc-linear-progress--indeterminate .mdc-linear-progress__secondary-bar{left:-54.888891%;display:block}.mdc-linear-progress--indeterminate.mdc-linear-progress--animation-ready .mdc-linear-progress__secondary-bar{animation:mdc-linear-progress-secondary-indeterminate-translate calc(2s*var(--mat-progress-bar-animation-multiplier)) infinite linear}.mdc-linear-progress--indeterminate.mdc-linear-progress--animation-ready .mdc-linear-progress__secondary-bar>.mdc-linear-progress__bar-inner{animation:mdc-linear-progress-secondary-indeterminate-scale calc(2s*var(--mat-progress-bar-animation-multiplier)) infinite linear}[dir=rtl] .mdc-linear-progress.mdc-linear-progress--animation-ready .mdc-linear-progress__secondary-bar{animation-name:mdc-linear-progress-secondary-indeterminate-translate-reverse}[dir=rtl] .mdc-linear-progress.mdc-linear-progress--indeterminate .mdc-linear-progress__secondary-bar{right:-54.888891%;left:auto}@keyframes mdc-linear-progress-buffering{from{transform:rotate(180deg) translateX(calc(var(--mat-progress-bar-track-height, 4px) * -2.5))}}@keyframes mdc-linear-progress-primary-indeterminate-translate{0%{transform:translateX(0)}20%{animation-timing-function:cubic-bezier(0.5, 0, 0.701732, 0.495819);transform:translateX(0)}59.15%{animation-timing-function:cubic-bezier(0.302435, 0.381352, 0.55, 0.956352);transform:translateX(83.67142%)}100%{transform:translateX(200.611057%)}}@keyframes mdc-linear-progress-primary-indeterminate-scale{0%{transform:scaleX(0.08)}36.65%{animation-timing-function:cubic-bezier(0.334731, 0.12482, 0.785844, 1);transform:scaleX(0.08)}69.15%{animation-timing-function:cubic-bezier(0.06, 0.11, 0.6, 1);transform:scaleX(0.661479)}100%{transform:scaleX(0.08)}}@keyframes mdc-linear-progress-secondary-indeterminate-translate{0%{animation-timing-function:cubic-bezier(0.15, 0, 0.515058, 0.409685);transform:translateX(0)}25%{animation-timing-function:cubic-bezier(0.31033, 0.284058, 0.8, 0.733712);transform:translateX(37.651913%)}48.35%{animation-timing-function:cubic-bezier(0.4, 0.627035, 0.6, 0.902026);transform:translateX(84.386165%)}100%{transform:translateX(160.277782%)}}@keyframes mdc-linear-progress-secondary-indeterminate-scale{0%{animation-timing-function:cubic-bezier(0.205028, 0.057051, 0.57661, 0.453971);transform:scaleX(0.08)}19.15%{animation-timing-function:cubic-bezier(0.152313, 0.196432, 0.648374, 1.004315);transform:scaleX(0.457104)}44.15%{animation-timing-function:cubic-bezier(0.257759, -0.003163, 0.211762, 1.38179);transform:scaleX(0.72796)}100%{transform:scaleX(0.08)}}@keyframes mdc-linear-progress-primary-indeterminate-translate-reverse{0%{transform:translateX(0)}20%{animation-timing-function:cubic-bezier(0.5, 0, 0.701732, 0.495819);transform:translateX(0)}59.15%{animation-timing-function:cubic-bezier(0.302435, 0.381352, 0.55, 0.956352);transform:translateX(-83.67142%)}100%{transform:translateX(-200.611057%)}}@keyframes mdc-linear-progress-secondary-indeterminate-translate-reverse{0%{animation-timing-function:cubic-bezier(0.15, 0, 0.515058, 0.409685);transform:translateX(0)}25%{animation-timing-function:cubic-bezier(0.31033, 0.284058, 0.8, 0.733712);transform:translateX(-37.651913%)}48.35%{animation-timing-function:cubic-bezier(0.4, 0.627035, 0.6, 0.902026);transform:translateX(-84.386165%)}100%{transform:translateX(-160.277782%)}}@keyframes mdc-linear-progress-buffering-reverse{from{transform:translateX(-10px)}}\n"],
			Ab: 2
		});
		_.tO = class {};
		_.tO.J = function(a) {
			return new (a || _.tO)();
		};
		_.tO.qc = _.Ve({ type: _.tO });
		_.tO.oc = _.Dd({ imports: [_.uA] });
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
		_.hr("qSZbi");
		var qhe = function(a, b, c) {
			return a === b ? 0 : (a < b ? -1 : 1) * (c ? 1 : -1);
		};
		var rhe = function(a, b) {
			b.sort((c, d) => {
				var e = a.direction === "asc";
				switch (a.active) {
					case "name": return qhe(c.getName().toLowerCase(), d.getName().toLowerCase(), e);
					case "description": return c.jc().trim() === "" ? 1 : d.jc().trim() === "" ? -1 : qhe(c.jc().toLowerCase(), d.jc().toLowerCase(), e);
					case "owner": return qhe(c.Aq().toLowerCase(), d.Aq().toLowerCase(), e);
					case "type": return qhe(c.getType(), d.getType(), e);
					case "updated": return qhe(c.iE(), d.iE(), e);
					default: return 0;
				}
			});
			return b;
		};
		var Cie = function(a) {
			a.querySelectorAll("[data-contenteditable=\"false\"]").forEach((b) => {
				b.setAttribute("contenteditable", "false");
				b.removeAttribute("data-contenteditable");
			});
			return a;
		};
		_.kn.prototype.P7 = _.ca(83, function() {
			return _.l(this, 8);
		});
		_.Cod.prototype.P7 = _.ca(82, function() {
			return ohe(this.prompt);
		});
		_.hx.prototype.vR = _.ca(57, function() {
			return _.l(this, 4);
		});
		_.Cod.prototype.vR = _.ca(56, function() {
			return _.Rnd(new Date(_.yod(this.prompt)));
		});
		_.Hv.prototype.bba = _.ca(29, function(a) {
			var b = document.implementation.createHTMLDocument("");
			if (this.H && this.F) {
				var c = document.createElement("safevalues-with-css");
				let d = c.attachShadow({ mode: "closed" });
				a = _.IVa(this, a, b);
				let e = document.createElement("style");
				e.textContent = ":host{display:inline-block;clip-path:inset(0);overflow:hidden;vertical-align:top;text-decoration:inherit}";
				e.id = "safevalues-internal-style";
				d.appendChild(e);
				d.appendChild(a);
				b = b.createDocumentFragment();
				b.appendChild(c);
				c = b;
			} else c = _.IVa(this, a, b);
			return c;
		});
		_.Vy.prototype.bba = _.ca(28, function(a) {
			a = this.cba.bba(_.ldb(a));
			return Cie(a);
		});
		var Die = /&([^;\s<&]+);?/g;
		var Eie = class {
			transform(a, b, c) {
				if (!a) return [];
				a = b ? a.filter((d) => {
					if (b == null || b === "") var e = true;
					else {
						e = d.getName().toLowerCase();
						var f = d.jc().toLowerCase();
						var g = d.P7().toLowerCase();
						var k = d.getType().toLowerCase();
						d = d.vR().toLowerCase();
						var p = b.toLowerCase();
						e = e.includes(p) || f.includes(p) || g.includes(p) || k.includes(p) || d.includes(p);
					}
					return e;
				}) : a;
				return rhe(c, [...a]);
			}
		};
		Eie.J = function(a) {
			return new (a || Eie)();
		};
		Eie.Wo = _.Xe({
			name: "libraryTablePipe",
			type: Eie,
			wk: true
		});
		var Fie = class {
			constructor() {
				this.query = _.M("");
				this.EHa = _.Ki();
				this.S = _.Dk;
			}
			oda(a) {
				a = a === null ? "" : String(a);
				this.query.set(a);
				this.EHa.emit(a);
			}
			Du() {
				this.query.set("");
				this.EHa.emit("");
			}
		};
		Fie.J = function(a) {
			return new (a || Fie)();
		};
		Fie.ka = _.u({
			type: Fie,
			da: [["ms-library-search-bar"]],
			outputs: { EHa: "queryChange" },
			ha: 2,
			ia: 5,
			la: [[
				"label",
				"Search",
				"hideLabel",
				"",
				"placeholder",
				"Search",
				3,
				"valueChange",
				"value",
				"icon"
			], [
				"ms-button",
				"",
				"variant",
				"icon-borderless",
				"aria-label",
				"Click to clear search query",
				1,
				"search-bar-close-button",
				3,
				"click",
				"iconName"
			]],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "ms-input-field", 0), _.J("valueChange", function(c) {
						return b.oda(c);
					}), _.F(1, "button", 1), _.J("click", function() {
						return b.Du();
					}), _.H()();
				}
				if (a & 2) {
					_.E("value", b.query())("icon", b.S.Lm), _.y(), _.P("show", b.query()), _.E("iconName", b.S.ac);
				}
			},
			dependencies: [_.Yy, _.mE],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;position:relative}.search-bar-close-button[_ngcontent-%COMP%]{visibility:hidden}.search-bar-close-button.show[_ngcontent-%COMP%]{visibility:visible}"]
		});
		var Gie = new _.$y("45779602", true);
		;
		var Hie = function(a) {
			var b = a.nx();
			switch (b) {
				case "me":
					a.IU.set(a.A().filter((c) => _.U3(c.prompt)));
					break;
				case "shared":
					a.IU.set(a.A().filter((c) => !_.U3(c.prompt)));
					break;
				case "all":
					a.IU.set(a.A());
					break;
				default: _.sb(b, undefined);
			}
		};
		var Iie = class {
			constructor() {
				this.R = _.m(_.sH);
				this.I = _.m(_.Op);
				this.dCa = this.I.getFlag(Gie);
				this.F = _.m(_.Ou);
				this.dialog = _.m(_.rC);
				this.U = _.m(_.Cl);
				this.H = _.m(_.rF);
				this.mG = _.m(_.V3);
				this.Wd = _.Ck(this.H.Xm, { initialValue: true });
				this.query = _.M("");
				this.S = _.Dk;
				this.IU = _.M([]);
				this.oMa = _.M([]);
				this.pUa = _.Ck(this.R.A.pipe(_.uf((a) => `https://drive.google.com/drive/folders/${a.getId()}`), _.Yg({
					bufferSize: 1,
					refCount: true
				})));
				this.Sa = this.mG.Sa;
				this.A = this.mG.A;
				this.X = _.m(_.ZC);
				this.Kh = this.X.A.Il;
				this.nx = this.mG.nx;
				this.yJb = [
					{
						value: "all",
						label: "All"
					},
					{
						value: "me",
						label: "My files"
					},
					{
						value: "shared",
						label: "Shared"
					}
				];
				this.fca = this.mG.fca;
				this.xAa = (a) => ({
					text: a.getName(),
					routerLink: a.vB(),
					af: _.Px(a.getPrompt()),
					icon: "chat_bubble"
				});
				this.wja = (a) => {
					var b = a.vB();
					return {
						Sfa: b.startsWith("/") ? b : "/" + b,
						state: { af: _.Px(a.getPrompt()) }
					};
				};
				this.dWa = "/prompts/new_chat";
				_.Fk([this.Wd], () => {
					if (this.Wd()) {
						_.Dod(this.mG);
					}
				});
				_.Fk([this.A], () => {
					var a;
					var b = (a = this.A()) != null ? a : [];
					this.oMa.set(b.filter((c) => _.U3(c.prompt)));
					Hie(this);
				});
				_.Fk([this.nx], () => {
					if (this.nx()) {
						Hie(this);
					}
				});
			}
			bna(a) {
				_.Rn(this.F, "LIBRARY", "Edited Search Input");
				this.query.set(a);
			}
			rga() {
				_.pF(this.H);
			}
			hO(a) {
				_.Rn(this.F, "LIBRARY", "Clicked Sort Prompt Table", a.active);
				this.mG.fca.set(a);
			}
		};
		Iie.J = function(a) {
			return new (a || Iie)();
		};
		Iie.ka = _.u({
			type: Iie,
			da: [["ms-library-table"]],
			Ua: 2,
			Ja: function(a, b) {
				if (a & 2) {
					_.P("v2", b.dCa);
				}
			},
			ha: 3,
			ia: 1,
			la: [
				["libraryview", ""],
				[1, "lib-view"],
				[1, "header-container"],
				[1, "header"],
				[1, "header-actions"],
				[
					"ariaLabel",
					"Select library view",
					3,
					"valueChange",
					"options",
					"value",
					"showLabelsOnMobile"
				],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"target",
					"_blank",
					"aria-label",
					"Open in Drive",
					3,
					"iconName",
					"href"
				],
				[3, "queryChange"],
				[1, "loading-state"],
				[1, "empty"],
				[1, "lib-table-wrapper"],
				[1, "prompt-cards-container"],
				[
					"mat-table",
					"",
					"matSort",
					"",
					"disableClear",
					"",
					1,
					"library-table",
					3,
					"matSortChange",
					"dataSource"
				],
				["matColumnDef", "icon"],
				[
					"mat-header-cell",
					"",
					"class",
					"icon-cell",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell icon-cell",
					3,
					"click",
					4,
					"matCellDef"
				],
				["matColumnDef", "name"],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"name",
					"class",
					"table-header-cell",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell name-cell",
					3,
					"click",
					4,
					"matCellDef"
				],
				["matColumnDef", "updated"],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"updated",
					"class",
					"table-header-cell updated-cell",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell updated-cell",
					3,
					"click",
					4,
					"matCellDef"
				],
				["matColumnDef", "overflow"],
				[
					"mat-header-cell",
					"",
					"class",
					"actions-cell",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell actions-cell",
					4,
					"matCellDef"
				],
				[
					"mat-header-row",
					"",
					4,
					"matHeaderRowDef"
				],
				[
					"mat-row",
					"",
					4,
					"matRowDef",
					"matRowDefColumns"
				],
				[
					"mat-header-cell",
					"",
					1,
					"icon-cell"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell",
					"icon-cell",
					3,
					"click"
				],
				[
					1,
					"prompt-icon",
					3,
					"iconName"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"name",
					1,
					"table-header-cell"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell",
					"name-cell",
					3,
					"click"
				],
				[
					1,
					"name-link",
					3,
					"click",
					"routerLink",
					"state",
					"title"
				],
				[
					1,
					"sub-text",
					"subtitle-line"
				],
				[1, "subtitle-separator"],
				[1, "subtitle-description"],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"updated",
					1,
					"table-header-cell",
					"updated-cell"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell",
					"updated-cell",
					3,
					"click"
				],
				[1, "sub-text"],
				[
					"mat-header-cell",
					"",
					1,
					"actions-cell"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell",
					"actions-cell"
				],
				[3, "promptItem"],
				["mat-header-row", ""],
				["mat-row", ""],
				[
					"role",
					"button",
					"tabindex",
					"0",
					1,
					"prompt-card"
				],
				[
					"role",
					"button",
					"tabindex",
					"0",
					1,
					"prompt-card",
					3,
					"click",
					"keydown.enter",
					"keydown.space"
				],
				[1, "card-header"],
				[1, "card-name-info"],
				[1, "card-name-column"],
				[
					1,
					"name-link",
					3,
					"click",
					"routerLink",
					"state"
				],
				[1, "card-actions"],
				[1, "card-details"],
				[1, "card-detail-row"],
				[1, "card-detail-label"],
				[1, "card-detail-value"],
				[
					1,
					"card-actions",
					3,
					"mousedown",
					"keydown"
				],
				[1, "loading-text"],
				[1, "empty-text"],
				[
					"ms-button",
					"",
					"aria-label",
					"Create new chat prompt",
					3,
					"iconName",
					"routerLink"
				],
				[
					"ms-button",
					"",
					"aria-label",
					"Allow Drive access",
					1,
					"auth-button",
					3,
					"click",
					"iconName"
				],
				[
					1,
					"lib-header",
					"flex-wrapper",
					"space-between"
				],
				[1, "title-wrapper"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"aria-label",
					"Select library view",
					3,
					"matMenuTriggerFor"
				],
				[1, "lib-header-title"],
				[
					1,
					"end-icon",
					"lib-table-dropdown-arrow",
					3,
					"iconName"
				],
				[
					"mat-menu-item",
					"",
					3,
					"click"
				],
				[
					1,
					"actions-wrapper",
					"flex-wrapper"
				],
				[
					"ms-button",
					"",
					"target",
					"_blank",
					1,
					"responsive-button-viewport-medium",
					"viewport-small-hidden",
					3,
					"iconName",
					"href"
				],
				[
					"mat-header-cell",
					"",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					3,
					"click",
					4,
					"matCellDef"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"name",
					"class",
					"table-header",
					4,
					"matHeaderCellDef"
				],
				["matColumnDef", "description"],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"description",
					"class",
					"table-header",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					3,
					"prompt-preview",
					"click",
					4,
					"matCellDef"
				],
				["matColumnDef", "owner"],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"owner",
					"class",
					"table-header",
					4,
					"matHeaderCellDef"
				],
				["matColumnDef", "type"],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"type",
					"class",
					"table-header",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"updated",
					"class",
					"table-header",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					4,
					"matCellDef"
				],
				["mat-header-cell", ""],
				[
					"mat-cell",
					"",
					3,
					"click"
				],
				[
					1,
					"margin-top",
					"prompt-icon",
					3,
					"iconName"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"name",
					1,
					"table-header"
				],
				[
					1,
					"tooltip-overflow",
					"name-btn",
					3,
					"click",
					"routerLink",
					"state",
					"title"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"description",
					1,
					"table-header"
				],
				[1, "tooltip-overflow"],
				[1, "preview-container"],
				[3, "iconName"],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"owner",
					1,
					"table-header"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"type",
					1,
					"table-header"
				],
				[1, "prompt-type"],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"updated",
					1,
					"table-header"
				],
				["mat-cell", ""],
				[3, "diameter"],
				[
					"ms-button",
					"",
					"aria-label",
					"Allow Drive access",
					1,
					"auth-button",
					"dark",
					3,
					"click",
					"iconName"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "section", 1), _.B(1, Xhe, 15, 15)(2, Bie, 49, 19), _.H();
				}
				if (a & 2) {
					_.y(), _.C(b.dCa ? 1 : 2);
				}
			},
			dependencies: [
				_.Yy,
				_.qI,
				_.tz,
				_.rI,
				_.dz,
				_.$D,
				_.wI,
				_.tI,
				_.sI,
				_.vI,
				_.TB,
				_.tO,
				_.zC,
				_.yC,
				_.VI,
				_.TI,
				_.UI,
				_.hJ,
				_.gJ,
				_.XI,
				_.aJ,
				_.YI,
				_.WI,
				_.bJ,
				_.ZI,
				_.$I,
				_.cJ,
				_.dJ,
				_.IC,
				_.W3,
				_.MD,
				_.sA,
				Fie,
				_.oz,
				Eie
			],
			styles: ["[_nghost-%COMP%]{-webkit-box-flex:1;-webkit-flex-grow:1;-moz-box-flex:1;-ms-flex-positive:1;flex-grow:1;height:100%}.v2[_nghost-%COMP%]   .lib-view[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;height:100%;padding-top:60px}.v2[_nghost-%COMP%]   .header-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-flow:row wrap;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-flow:row wrap;flex-flow:row wrap;gap:8px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;margin-bottom:32px}.v2[_nghost-%COMP%]   .header[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:24px;font-weight:600;line-height:32px;white-space:nowrap}.v2[_nghost-%COMP%]   .header-actions[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-flow:row nowrap;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-flow:row nowrap;flex-flow:row nowrap;gap:8px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;margin-left:auto}@media screen and (max-width:600px){.v2[_nghost-%COMP%]   ms-library-search-bar[_ngcontent-%COMP%]{-webkit-box-flex:1;-webkit-flex:1 0 100%;-moz-box-flex:1;-ms-flex:1 0 100%;flex:1 0 100%}}.v2[_nghost-%COMP%]   .lib-table-wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-flex:1;-webkit-flex-grow:1;-moz-box-flex:1;-ms-flex-positive:1;flex-grow:1;margin-left:-24px;margin-right:-24px}@media screen and (max-width:600px){.v2[_nghost-%COMP%]   .lib-table-wrapper[_ngcontent-%COMP%]{margin-left:-16px;margin-right:-16px}}.v2[_nghost-%COMP%]   .mat-mdc-row[_ngcontent-%COMP%]   .mat-mdc-cell[_ngcontent-%COMP%]{padding-top:12px;padding-bottom:12px}.v2[_nghost-%COMP%]   .mat-mdc-header-row[_ngcontent-%COMP%]{height:40px}.v2[_nghost-%COMP%]   .mat-mdc-header-row[_ngcontent-%COMP%]   .mat-mdc-header-cell[_ngcontent-%COMP%]:first-child{padding-left:24px}.v2[_nghost-%COMP%]   .mat-mdc-header-row[_ngcontent-%COMP%]   .mat-mdc-header-cell[_ngcontent-%COMP%]:last-child{padding-right:24px}.v2[_nghost-%COMP%]   .table-header-cell[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text-var)}.v2[_nghost-%COMP%]   .table-body-cell[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}.v2[_nghost-%COMP%]   .sub-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}.v2[_nghost-%COMP%]   .library-table[_ngcontent-%COMP%]{table-layout:fixed;width:100%;border-collapse:collapse;border-spacing:0}.v2[_nghost-%COMP%]   .library-table[_ngcontent-%COMP%]   tr.mat-mdc-row[_ngcontent-%COMP%]{-webkit-transition:background-color .15s ease-in-out;transition:background-color .15s ease-in-out}.v2[_nghost-%COMP%]   .library-table[_ngcontent-%COMP%]   tr.mat-mdc-row[_ngcontent-%COMP%]   td[_ngcontent-%COMP%]{border-bottom:1px solid var(--color-v3-outline-var)}.v2[_nghost-%COMP%]   .library-table[_ngcontent-%COMP%]   tr.mat-mdc-row[_ngcontent-%COMP%]:hover{background:var(--color-v3-surface-container-highest)}.v2[_nghost-%COMP%]   .library-table[_ngcontent-%COMP%]   tr.mat-mdc-row[_ngcontent-%COMP%]:hover   td[_ngcontent-%COMP%]:first-child{border-radius:12px 0 0 12px}.v2[_nghost-%COMP%]   .library-table[_ngcontent-%COMP%]   tr.mat-mdc-row[_ngcontent-%COMP%]:hover   td[_ngcontent-%COMP%]:last-child{border-radius:0 12px 12px 0}.v2[_nghost-%COMP%]   .library-table[_ngcontent-%COMP%]   tr.mat-mdc-row[_ngcontent-%COMP%]   td[_ngcontent-%COMP%]:first-child{padding-left:24px}.v2[_nghost-%COMP%]   .library-table[_ngcontent-%COMP%]   tr.mat-mdc-row[_ngcontent-%COMP%]   td[_ngcontent-%COMP%]:last-child{padding-right:24px}.v2[_nghost-%COMP%]   .icon-cell[_ngcontent-%COMP%]{width:58px;vertical-align:middle}.v2[_nghost-%COMP%]   .name-cell[_ngcontent-%COMP%]{vertical-align:top}.v2[_nghost-%COMP%]   .name-cell[_ngcontent-%COMP%]   .name-link[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text);display:-webkit-box;-webkit-line-clamp:1;-webkit-box-orient:vertical;overflow:hidden;text-decoration:none}.v2[_nghost-%COMP%]   .name-cell[_ngcontent-%COMP%]   .name-link[_ngcontent-%COMP%]:hover{text-decoration:underline}.v2[_nghost-%COMP%]   .name-cell[_ngcontent-%COMP%]   .subtitle-line[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:baseline;-webkit-align-items:baseline;-moz-box-align:baseline;-ms-flex-align:baseline;align-items:baseline;gap:4px;margin-top:2px;line-height:1.3;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.v2[_nghost-%COMP%]   .name-cell[_ngcontent-%COMP%]   .subtitle-separator[_ngcontent-%COMP%]{-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;opacity:.5}.v2[_nghost-%COMP%]   .name-cell[_ngcontent-%COMP%]   .subtitle-description[_ngcontent-%COMP%]{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}.v2[_nghost-%COMP%]   .updated-cell[_ngcontent-%COMP%]{white-space:nowrap;width:120px}.v2[_nghost-%COMP%]   .actions-cell[_ngcontent-%COMP%]{width:58px;vertical-align:middle}.v2[_nghost-%COMP%]   .loading-state[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:12px;padding-top:60px}.v2[_nghost-%COMP%]   .loading-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}.v2[_nghost-%COMP%]   .empty[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-flex:1;-webkit-flex-grow:1;-moz-box-flex:1;-ms-flex-positive:1;flex-grow:1;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:24px 0}.v2[_nghost-%COMP%]   .empty[_ngcontent-%COMP%]   .empty-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}.v2[_nghost-%COMP%]   .auth-button[_ngcontent-%COMP%]{margin-top:12px}.v2[_nghost-%COMP%]   .prompt-cards-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:12px}.v2[_nghost-%COMP%]   .prompt-card[_ngcontent-%COMP%]{border:1px solid var(--color-v3-outline-var);border-radius:12px;padding:16px;background:var(--color-v3-surface-container-high);cursor:pointer;-webkit-transition:background-color .15s ease-in-out;transition:background-color .15s ease-in-out}.v2[_nghost-%COMP%]   .prompt-card.cdk-keyboard-focused[_ngcontent-%COMP%], .v2[_nghost-%COMP%]   .prompt-card.cdk-touch-focused[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container-highest)}@media (hover:hover),(pointer:none){.v2[_nghost-%COMP%]   .prompt-card[_ngcontent-%COMP%]:hover{background-color:var(--color-v3-surface-container-highest)}}.v2[_nghost-%COMP%]   .card-header[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px}.v2[_nghost-%COMP%]   .card-name-info[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;min-width:0;-webkit-box-flex:1;-webkit-flex:1;-moz-box-flex:1;-ms-flex:1;flex:1}.v2[_nghost-%COMP%]   .card-name-column[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:2px;min-width:0}.v2[_nghost-%COMP%]   .card-name-column[_ngcontent-%COMP%]   .name-link[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text);display:-webkit-box;-webkit-line-clamp:1;-webkit-box-orient:vertical;overflow:hidden;text-decoration:none}.v2[_nghost-%COMP%]   .card-name-column[_ngcontent-%COMP%]   .name-link[_ngcontent-%COMP%]:hover{text-decoration:underline}.v2[_nghost-%COMP%]   .card-actions[_ngcontent-%COMP%]{-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.v2[_nghost-%COMP%]   .card-details[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px;margin-top:12px;padding-top:12px;border-top:1px solid var(--color-v3-outline-var)}.v2[_nghost-%COMP%]   .card-detail-row[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px}.v2[_nghost-%COMP%]   .card-detail-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var);-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.v2[_nghost-%COMP%]   .card-detail-value[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text);text-align:end;min-width:0}[_nghost-%COMP%]:not(.v2)   .lib-view[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;height:100%;padding-top:60px}[_nghost-%COMP%]:not(.v2)   .lib-view[_ngcontent-%COMP%]   .lib-table-dropdown-arrow[_ngcontent-%COMP%], [_nghost-%COMP%]:not(.v2)   .lib-view[_ngcontent-%COMP%]   h3[_ngcontent-%COMP%]{color:var(--color-white)}[_nghost-%COMP%]:not(.v2)   .lib-header[_ngcontent-%COMP%]{background:transparent;padding-bottom:12px;padding-top:24px;position:-webkit-sticky;position:sticky;top:0;z-index:1;-webkit-backdrop-filter:blur(5px);backdrop-filter:blur(5px)}[_nghost-%COMP%]:not(.v2)   .title-wrapper[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:4px;height:-webkit-max-content;height:-moz-max-content;height:max-content}[_nghost-%COMP%]:not(.v2)   .title-wrapper[_ngcontent-%COMP%]   .lib-header-title[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:16px;font-weight:600;line-height:24px}[_nghost-%COMP%]:not(.v2)   .title-wrapper[_ngcontent-%COMP%]   .lib-table-dropdown-arrow[_ngcontent-%COMP%]{padding-top:4px}[_nghost-%COMP%]:not(.v2)   .lib-table-wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-flex:1;-webkit-flex-grow:1;-moz-box-flex:1;-ms-flex-positive:1;flex-grow:1;overflow-x:auto}[_nghost-%COMP%]:not(.v2)   .content[_ngcontent-%COMP%]{white-space:pre-wrap}[_nghost-%COMP%]:not(.v2)   .empty[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-flex:1;-webkit-flex-grow:1;-moz-box-flex:1;-ms-flex-positive:1;flex-grow:1;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:24px 0}[_nghost-%COMP%]:not(.v2)   .empty[_ngcontent-%COMP%]   .empty-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px}[_nghost-%COMP%]:not(.v2)   .empty[_ngcontent-%COMP%]   img[_ngcontent-%COMP%]{margin:12px 0}[_nghost-%COMP%]:not(.v2)   .library-table[_ngcontent-%COMP%]   tr[_ngcontent-%COMP%]:has(td):focus, [_nghost-%COMP%]:not(.v2)   .library-table[_ngcontent-%COMP%]   tr[_ngcontent-%COMP%]:has(td):hover{background:var(--color-inverse-on-surface)}[_nghost-%COMP%]:not(.v2)   .name-btn[_ngcontent-%COMP%]{color:var(--color-v3-text-on-button);display:block;max-width:200px;min-width:100px;overflow:hidden;text-align:start}[_nghost-%COMP%]:not(.v2)   .tooltip-overflow[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex}[_nghost-%COMP%]:not(.v2)   .prompt-preview[_ngcontent-%COMP%]{color:var(--color-v3-text-var);font-style:italic}[_nghost-%COMP%]:not(.v2)   .preview-container[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;gap:4px}[_nghost-%COMP%]:not(.v2)   .auth-button[_ngcontent-%COMP%]{margin-top:12px}[_nghost-%COMP%]:not(.v2)   .link-button-content[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}"]
		});
		_.ys = class {};
		_.ys.J = function(a) {
			return new (a || _.ys)();
		};
		_.ys.ka = _.u({
			type: _.ys,
			da: [["ms-library"]],
			ha: 4,
			ia: 0,
			la: [
				[
					1,
					"page-content-wrapper",
					"inner-padding"
				],
				[1, "padding-wrapper"],
				[1, "page-content-inner-wrapper"]
			],
			template: function(a) {
				if (a & 1) {
					_.F(0, "div", 0)(1, "div", 1)(2, "div", 2), _.I(3, "ms-library-table"), _.H()()();
				}
			},
			dependencies: [Iie],
			styles: [".padding-wrapper[_ngcontent-%COMP%]{padding-left:24px;padding-right:24px;height:100%}"]
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

