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
		_.k1 = class {
			constructor() {
				this.YRb = "https://www.gstatic.com/aistudio/icons/spark_icon_traces.svg";
			}
		};
		_.k1.J = function(a) {
			return new (a || _.k1)();
		};
		_.k1.ka = _.u({
			type: _.k1,
			da: [["ms-sparkle-icon"]],
			ha: 1,
			ia: 1,
			la: [[
				"alt",
				"",
				3,
				"src"
			]],
			template: function(a, b) {
				if (a & 1) {
					_.Fh(0, "img", 0);
				}
				if (a & 2) {
					_.Ch("src", b.YRb, _.rg);
				}
			},
			styles: ["img[_ngcontent-%COMP%]{height:100%;object-fit:contain}"]
		});
		_.Kjd = function() {
			return _.Jn().location.hostname.includes("aistudio-dev.corp.google.com");
		};
		var Mjd;
		Ljd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 14);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.rz());
				});
				_.Mh(1, 1);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("disabled", a.Hb)("xapInlineDialog", a.Hb ? a.Ul : null)("ve", a.uMa().Y9a)("veClick", true)("veImpression", true);
			}
		};
		Njd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 15);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					_.Rn(c.dPa, "API", "Clicked Activate billing button on a Project");
					Mjd(c, "61cf5150-677c-4f21-9d62-6edc44b0a46b");
					return _.t();
				});
				_.Mh(1, 2);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("disabled", a.Hb)("xapInlineDialog", a.Hb ? a.Ul : null)("ve", a.uMa().uTa)("veClick", true)("veImpression", true);
			}
		};
		Ojd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 16);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					if (c.YEb()) {
						Mjd(c);
					} else {
						c.jna();
					}
					return _.t();
				});
				_.Mh(1, 3);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.P("disabled", a.Hb), _.E("disabled", a.Hb)("xapInlineDialog", a.Hb ? a.Ul : null);
			}
		};
		Pjd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 17);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.jna());
				});
				_.Mh(1, 4);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("title", a.XUa()), _.y(), _.Qh(a.XUa()), _.Rh(1);
			}
		};
		Qjd = function(a) {
			if (a & 1) {
				_.F(0, "span", 12), _.Gh(1), _.Mh(2, 5), _.Hh(), _.H();
			}
			if (a & 2) {
				a = _.K();
				let b = _.O(9);
				_.E("xapInlineDialog", b);
				_.y(2);
				_.Qh(a.fUb());
				_.Rh(2);
			}
		};
		Rjd = function(a) {
			if (a & 1) {
				_.R(0);
			}
			if (a & 2) {
				a = _.K(), _.S(" · ", a.YUa(), " ");
			}
		};
		Sjd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "div", 18)(1, "div");
				_.Kh(2, 6);
				_.F(3, "a", 19);
				_.I(4, "span", 20);
				_.H();
				_.Lh();
				_.H();
				_.F(5, "div", 21);
				_.Gh(6, 21);
				_.Mh(7, 7);
				_.Hh();
				_.F(8, "button", 22);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.Dxa());
				});
				_.H()()();
			}
			if (a & 2) {
				a = _.K(), _.y(4), _.E("iconName", a.S.Dk), _.y(3), _.Qh(a.billingAccountId()), _.Rh(7), _.y(), _.E("iconName", a.S.HOa);
			}
		};
		Tjd = function(a) {
			if (a & 1) {
				_.F(0, "span"), _.R(1, ","), _.H();
			}
		};
		Vjd = function(a, b) {
			if (a & 1) {
				let c = _.n();
				_.F(0, "button", 3);
				_.J("click", function() {
					var d = _.q(c).V;
					var e = _.K(2);
					return _.t(e.jna(d));
				});
				_.R(1);
				_.H();
				_.B(2, Tjd, 2, 0, "span");
			}
			if (a & 2) {
				a = b.V;
				let c = b.jb;
				b = b.lj;
				let d = _.K(2);
				_.E("ve", d.ve.Mob)("veClick", true)("veImpression", true);
				_.y();
				_.S(" ", Ujd(d, a.getDisplayName()), " ");
				_.y();
				_.C(c !== b - 1 ? 2 : -1);
			}
		};
		Xjd = function(a) {
			if (a & 1) {
				_.F(0, "ms-callout", 0)(1, "div", 1)(2, "span"), _.R(3), _.H(), _.F(4, "div", 2), _.Ah(5, Vjd, 3, 5, null, null, Wjd), _.H()()();
			}
			if (a & 2) {
				a = _.K(), _.y(3), _.U(a.calloutMessage()), _.y(2), _.Bh(a.W_());
			}
		};
		Mjd = function(a, b) {
			a.dialog.open(_.MG, {
				id: "oaas-dialog",
				data: {
					experienceId: b,
					st: a.project()
				}
			});
		};
		_.l3 = class {
			constructor() {
				this.project = _.Li.required();
				this.uMa = _.Li.required();
				this.dPa = _.m(_.Ou);
				this.R = _.m(_.GG);
				this.dialog = _.m(_.rC);
				this.U = _.m(_.SC);
				this.I = _.m(_.iC);
				this.aa = _.m(_.Cl);
				this.X = _.m(_.Qu);
				this.S = _.Dk;
				this.Ul = _.OG;
				this.Hb = _.Nn(this.X.Oe);
				_.W(() => "You do not have permission to view this account. Contact your billing admin to proceed.");
				this.H = _.W(() => _.au(this.project()));
				this.F = _.W(() => {
					var a;
					return (a = this.H()) == null ? undefined : a.gk();
				});
				this.A = _.W(() => {
					var a;
					return this.R.U().get((a = this.F()) != null ? a : "");
				});
				this.bBa = _.W(() => !!this.A());
				this.billingAccountId = _.W(() => {
					var a = this.F();
					return a ? _.Pn(a) : "";
				});
				this.XUa = _.W(() => this.A() ? this.A().getDisplayName() : "Billing account");
				this.fUb = _.W(() => {
					var a = this.billingAccountId();
					return `...${a.substring(a.length - 4)}`;
				});
				this.LH = _.W(() => !!this.F());
				this.YGb = _.W(() => {
					var a;
					if (a = !!this.A() && this.A().RY() !== 1) {
						a = this.A();
						a = _.Lm(a, 12) === 2;
					}
					return a;
				});
				this.YEb = _.W(() => {
					var a;
					var b;
					var c;
					return (c = (a = this.A()) == null ? undefined : (b = a.Ap()) == null ? undefined : b.includes(2)) != null ? c : false;
				});
				this.UAa = _.W(() => {
					var a;
					return ((a = this.H()) == null ? undefined : _.Lm(a, 3)) === 1;
				});
				this.oNb = _.W(() => {
					if (this.UAa()) return "Free trial";
					var a;
					return _.iya((a = this.project().ah()) != null ? a : 0);
				});
				this.YUa = _.W(() => {
					if (!this.A()) return "";
					var a;
					if (this.UAa() || ((a = this.H()) == null ? undefined : _.Lm(a, 3)) === 2) return "";
					a = this.A().RY();
					switch (a) {
						case 1: return "Prepay";
						case 2: return "Postpay";
						case 0: return "";
						default: _.sb(a, undefined);
					}
				});
			}
			rz() {
				_.Rn(this.dPa, "API", "Clicked Set up billing button on a Project");
				Mjd(this);
			}
			jna() {
				this.aa.navigate(["billing"], { queryParams: { billing: this.billingAccountId() } });
			}
			Dxa() {
				var a = this.U.copy(this.billingAccountId());
				var b = a ? "Copied to clipboard" : "Error copying to clipboard";
				if (a) {
					this.I.success(b);
				} else {
					this.I.error(b);
				}
			}
		};
		_.l3.J = function(a) {
			return new (a || _.l3)();
		};
		_.l3.ka = _.u({
			type: _.l3,
			da: [["ms-quota-tier-cell"]],
			inputs: {
				project: [1, "project"],
				uMa: [1, "veMap"]
			},
			ha: 10,
			ia: 3,
			la: () => [
				["permissionDeniedDialogTemplate", ""],
				" Set up billing ",
				" Activate billing ",
				" Set up prepay ",
				" �0� ",
				" Account (�0�) ",
				" You don't have sufficient read permissions for this billing account. Learn more about �#3� billing permissions �#4��/#4��/#3�",
				" Billing account ID: �0� ",
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-set-up-billing-link",
					"",
					1,
					"link",
					3,
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-activate-billing-button",
					"",
					1,
					"link",
					3,
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-set-up-prepay-button",
					"",
					1,
					"link",
					3,
					"disabled",
					"xapInlineDialog"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"View billing",
					"data-test-billing-account-name-button",
					"",
					1,
					"link",
					"billing-account-name-link",
					3,
					"title"
				],
				[
					"dialogLabel",
					"Billing permission error dialog",
					"data-test-billing-account-name-button-no-permission",
					"",
					1,
					"disabled",
					3,
					"xapInlineDialog"
				],
				[
					"data-test-quota-tier-text",
					"",
					1,
					"sub-text"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-set-up-billing-link",
					"",
					1,
					"link",
					3,
					"click",
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-activate-billing-button",
					"",
					1,
					"link",
					3,
					"click",
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"data-test-set-up-prepay-button",
					"",
					1,
					"link",
					3,
					"click",
					"disabled",
					"xapInlineDialog"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"View billing",
					"data-test-billing-account-name-button",
					"",
					1,
					"link",
					"billing-account-name-link",
					3,
					"click",
					"title"
				],
				[1, "permission-denied-dialog-content"],
				[
					"href",
					"https://docs.cloud.google.com/billing/docs/how-to/grant-access-to-billing",
					"target",
					"_blank",
					"rel",
					"noopener",
					1,
					"permission-denied-dialog-link"
				],
				[3, "iconName"],
				[1, "billing-account-id"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Copy billing account ID",
					"data-test-copy-billing-account-id",
					"",
					3,
					"click",
					"iconName"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, Ljd, 2, 5, "button", 8)(1, Njd, 2, 5, "button", 9)(2, Ojd, 2, 4, "button", 10)(3, Pjd, 2, 2, "button", 11)(4, Qjd, 3, 2, "span", 12), _.F(5, "div", 13), _.R(6), _.B(7, Rjd, 1, 1), _.H(), _.z(8, Sjd, 9, 3, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					_.C(b.LH() ? b.UAa() ? 1 : b.YGb() ? 2 : b.bBa() ? 3 : 4 : 0), _.y(6), _.S(" ", b.oNb(), " "), _.y(), _.C(b.YUa() ? 7 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.dz,
				_.IC,
				_.Bz,
				_.EC
			],
			styles: [".link[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;cursor:pointer;padding:0;margin:0;border:none;height:24px}.sub-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}.billing-account-name-link[_ngcontent-%COMP%]{display:inline-block;max-width:180px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;text-align:left}.disabled[_ngcontent-%COMP%]{color:var(--color-v3-text-disable)}.permission-denied-dialog-content[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;padding:16px}.permission-denied-dialog-content[_ngcontent-%COMP%]   .billing-account-id[_ngcontent-%COMP%], .permission-denied-dialog-content[_ngcontent-%COMP%]   .permission-denied-dialog-link[_ngcontent-%COMP%]{display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:4px}.permission-denied-dialog-content[_ngcontent-%COMP%]   .billing-account-id[_ngcontent-%COMP%]{margin-top:8px}"]
		});
		var Ujd;
		Wjd = (a, b) => b.Ya();
		Ujd = function(a, b) {
			return b.length <= 10 || a.W_().length === 1 || a.W_().length < 3 && !a.I.A.small() ? b : b.substring(0, 10) + "...";
		};
		_.m3 = class {
			constructor() {
				this.ve = { Mob: 311861 };
				this.F = _.m(_.GG);
				this.Za = _.m(_.Iy);
				this.H = _.m(_.Cl);
				this.I = _.m(_.ZC);
				this.r$a = _.M(false);
				this.Sd = this.Za.Sd;
				this.A = this.F.A;
				this.W_ = _.W(() => this.Sd().filter((a) => {
					var b;
					a = (b = _.au(a)) == null ? undefined : b.gk();
					if (!a) return false;
					var c;
					var d;
					return (d = (c = this.A().get(a)) == null ? undefined : c.has("PREPAY_PAYMENT_ACTION_NEEDED")) != null ? d : false;
				}));
				this.calloutMessage = _.W(() => new _.xd("{numProjects, plural, =1 {Your project has issues needing your attention.}other {Your projects have issues needing your attention.}}").format({ numProjects: this.W_().length }));
				_.Fk([this.W_], () => {
					this.r$a.set(this.W_().length > 0);
				});
			}
			jna(a) {
				var b = { queryParams: {} };
				var c;
				if (a = (c = _.au(a)) == null ? undefined : c.gk()) {
					b.queryParams.billing = _.Pn(a);
				}
				this.H.navigate(["billing"], b);
			}
		};
		_.m3.J = function(a) {
			return new (a || _.m3)();
		};
		_.m3.ka = _.u({
			type: _.m3,
			da: [["ms-payment-alert-callout"]],
			ha: 1,
			ia: 1,
			la: [
				[
					"calloutType",
					"error",
					1,
					"callout"
				],
				[
					"callout-content",
					"",
					1,
					"callout-content"
				],
				[1, "project-links"],
				[
					"ms-button",
					"",
					"variant",
					"link",
					1,
					"project-link",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, Xjd, 7, 1, "ms-callout", 0);
				}
				if (a & 2) {
					_.C(b.r$a() ? 0 : -1);
				}
			},
			dependencies: [
				_.zA,
				_.Yy,
				_.Cz,
				_.Bz
			],
			styles: [".callout[_ngcontent-%COMP%]{margin-bottom:20px}.project-links[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:4px;-webkit-box-align:end;-webkit-align-items:flex-end;-moz-box-align:end;-ms-flex-align:end;align-items:flex-end;-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap;margin-top:6px}.project-link[_ngcontent-%COMP%]{height:20px;padding:0}"]
		});
		Zjd = function(a) {
			if (a & 1) {
				_.I(0, "ms-sparkle-icon");
			}
		};
		$jd = function(a) {
			if (a & 1) {
				_.F(0, "a", 10), _.Mh(1, 0), _.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("ve", a.ve.xta)("veClick", true)("veImpression", true);
			}
		};
		akd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 12);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.ZE());
				});
				_.Mh(1, 1);
				_.H();
			}
			if (a & 2) {
				a = _.K(2), _.E("ve", a.ve.wta)("veClick", true)("veImpression", true);
			}
		};
		bkd = function(a) {
			if (a & 1) {
				_.B(0, $jd, 2, 3, "a", 10)(1, akd, 2, 3, "button", 11);
			}
			if (a & 2) {
				a = _.K(), _.C(a.Hb ? 0 : 1);
			}
		};
		ckd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 13);
				_.J("click", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.vr(true));
				});
				_.Mh(1, 2);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("ve", a.ve.yta)("veClick", true)("veImpression", true);
			}
		};
		dkd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "ms-import-projects-panel", 14);
				_.J("onClose", function() {
					_.q(b);
					var c = _.K();
					return _.t(c.vr(false));
				});
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("isRightPanelOpen", a.Jq());
			}
		};
		ekd = [[[
			"",
			"empty-state-buttons",
			""
		]]];
		_.n3 = class {
			constructor() {
				this.ve = {
					wta: 278932,
					xta: 278935,
					yta: 278936
				};
				this.H7a = _.Ki();
				this.F = _.m(_.Qu);
				this.dialog = _.m(_.rC);
				this.A = _.m(_.Ou);
				this.headline = _.Li.required();
				this.message = _.Li.required();
				this.SB = _.Li.required();
				this.V$a = _.Li.required();
				this.WJa = _.Li.required();
				this.v$a = _.Li.required();
				this.Hb = _.Nn(this.F.Oe);
				this.Jq = _.M(false);
				this.Ymb = "Learn more";
			}
			vr(a) {
				if (a) {
					_.Rn(this.A, "API", "Clicked Import Projects Button in project zero state");
				}
				this.Jq.set(a);
			}
			ZE() {
				var a = this;
				return _.x(function* () {
					_.Rn(a.A, "API", "Clicked Create Project Button in project zero state");
					if (yield _.pf(_.jC(a.dialog.open(_.tE)))) {
						a.H7a.emit();
					}
				});
			}
		};
		_.n3.J = function(a) {
			return new (a || _.n3)();
		};
		_.n3.ka = _.u({
			type: _.n3,
			da: [["ms-project-zero-state"]],
			inputs: {
				headline: [1, "headline"],
				message: [1, "message"],
				SB: [1, "learnMoreUrl"],
				V$a: [1, "showSparkle"],
				WJa: [1, "showImportProjectsButton"],
				v$a: [1, "showCreateProjectButton"]
			},
			outputs: { H7a: "projectCreated" },
			fc: ["[empty-state-buttons]"],
			ha: 13,
			ia: 8,
			la: () => [
				" Create a new project in GCP ",
				" Create project ",
				" Import projects ",
				[1, "empty-state-container"],
				[1, "empty-state-headline"],
				[1, "empty-state-text"],
				[
					"target",
					"_blank",
					3,
					"href"
				],
				[1, "empty-state-buttons"],
				[
					"ms-button",
					"",
					"size",
					"large",
					"data-test-id",
					"import-projects-button",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[3, "isRightPanelOpen"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"href",
					"http://go/aistudio-apikey#create-gcp-project",
					"target",
					"_blank",
					"matTooltip",
					"Google Internal Only",
					"matTooltipPosition",
					"above",
					"aria-label",
					"Create a new project in GCP",
					1,
					"custom-link",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"variant",
					"borderless",
					"data-test-id",
					"create-project-button",
					"aria-label",
					"Create a new project",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"variant",
					"borderless",
					"data-test-id",
					"create-project-button",
					"aria-label",
					"Create a new project",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"data-test-id",
					"import-projects-button",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					3,
					"onClose",
					"isRightPanelOpen"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Xh(ekd), _.F(0, "div", 3), _.B(1, Zjd, 1, 0, "ms-sparkle-icon"), _.F(2, "p", 4), _.R(3), _.H(), _.F(4, "p", 5), _.R(5), _.F(6, "a", 6), _.R(7), _.H()(), _.F(8, "div", 7), _.B(9, bkd, 2, 1), _.B(10, ckd, 2, 3, "button", 8), _.Yh(11), _.H()(), _.B(12, dkd, 1, 1, "ms-import-projects-panel", 9);
				}
				if (a & 2) {
					_.y(), _.C(b.V$a() ? 1 : -1), _.y(2), _.U(b.headline()), _.y(2), _.S(" ", b.message(), " "), _.y(), _.E("href", b.SB(), _.rg), _.y(), _.U(b.Ymb), _.y(2), _.C(b.v$a() ? 9 : -1), _.y(), _.C(b.WJa() ? 10 : -1), _.y(2), _.C(b.WJa() ? 12 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.rE,
				_.IC,
				_.HC,
				_.k1,
				_.Bz
			],
			styles: [".empty-state-container[_ngcontent-%COMP%]{max-width:400px;margin:0 auto;padding:32px 0;text-align:center}.empty-state-headline[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text);margin-top:36px;margin-bottom:8px}ms-sparkle-icon[_ngcontent-%COMP%] + .empty-state-headline[_ngcontent-%COMP%]{margin-top:8px}.empty-state-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var);margin-bottom:8px}.empty-state-buttons[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;gap:8px;margin-top:36px}.custom-link[_ngcontent-%COMP%]{color:var(--color-v3-text)}"]
		});
		_.hr("DBAUvd");
		_.Iy.prototype.L1 = _.ca(137, function(a) {
			var b = this;
			return _.x(function* () {
				try {
					var c = new _.D_a();
					var d = _.ln(c, _.Zt, 1, a);
					var e = b.H;
					yield _.$q(e.A, e.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/UpdateCloudProject", d, {}, _.Abb);
					b.I.delete(a.getName());
					b.R.reload();
				} catch (f) {
					throw _.Rn(b.U, "API", "Project Update Failed"), f;
				}
			});
		});
		var hCe = function(a, b) {
			return _.Xm(a, 1, b);
		};
		var iCe = function(a, b) {
			return _.$q(a.A, a.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/RemoveProjects", b, {}, _.Iab);
		};
		var jCe = function(a, b) {
			return _.x(function* () {
				try {
					let c = hCe(new _.C_a(), b);
					yield iCe(a.H, c);
					a.F.update((d) => (d != null ? d : []).filter((e) => !b.includes(e.getName())));
					for (let d of b) a.I.delete(d);
				} catch (c) {
					throw _.Rn(a.U, "API", "Project Removal Failed"), c;
				}
			});
		};
		var wBe = class {
			constructor() {
				this.Wa = _.m(_.kC);
				this.data = _.m(_.qC);
				this.dialog = _.m(_.rC);
				this.Za = _.m(_.Iy);
				this.A = _.m(_.Ou);
				this.project = _.M();
				this.tla = _.M(false);
				this.S = _.Dk;
				this.ys = _.W(() => {
					var a;
					return (a = this.project()) == null ? undefined : a.getDisplayName();
				});
				this.projectName = _.W(() => {
					var a;
					return (a = this.project()) == null ? undefined : a.getName();
				});
				this.projectId = _.W(() => {
					var a;
					return (a = this.project()) == null ? undefined : a.Ya();
				});
				this.projectNumber = _.W(() => {
					var a;
					return (a = this.projectName()) == null ? undefined : a.split("/")[1];
				});
				this.project.set(this.data.project);
			}
			removeProject() {
				var a = this;
				return _.x(function* () {
					a.tla.set(true);
					try {
						_.Rn(a.A, "API", "Clicked Remove Project Button in project details dialog");
						yield jCe(a.Za, [a.project().getName()]);
					} catch (b) {
						_.Rn(a.A, "API", "Project Removal Failed");
						console.error("Failed to remove project:", b);
					} finally {
						a.tla.set(false);
						a.Wa.close();
					}
				});
			}
			Bqa() {
				_.Rn(this.A, "API", "Clicked Copy Project Name Button in project details dialog");
			}
			QLa() {
				_.Rn(this.A, "API", "Clicked Copy Project Id Button in details dialog");
			}
			RLa() {
				_.Rn(this.A, "API", "Clicked Copy Project Number Button in project details dialog");
			}
		};
		wBe.J = function(a) {
			return new (a || wBe)();
		};
		wBe.ka = _.u({
			type: wBe,
			da: [["ms-project-details-dialog"]],
			ha: 8,
			ia: 3,
			la: () => [
				"Project details",
				"Name",
				"Project name",
				"Project number",
				"Project id",
				" Remove project ",
				" Copy name ",
				[1, "header"],
				[1, "title"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"matDialogClose",
					"",
					"aria-label",
					"close",
					1,
					"close-button",
					3,
					"iconName"
				],
				[1, "details-container"],
				[1, "spinner-container"],
				[3, "diameter"],
				[1, "section-wrapper"],
				[1, "field-header"],
				[1, "value-wrapper"],
				[1, "field-value"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Copy project display name to clipboard",
					3,
					"click",
					"iconName",
					"xapCopyToClipboard"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Copy project name to clipboard",
					3,
					"click",
					"iconName",
					"xapCopyToClipboard"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Copy project number to clipboard",
					3,
					"click",
					"iconName",
					"xapCopyToClipboard"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Copy project id to clipboard",
					3,
					"click",
					"iconName",
					"xapCopyToClipboard"
				],
				[
					"variant",
					"borderless",
					"ms-button",
					"",
					3,
					"click",
					"disabled"
				],
				[
					"ms-button",
					"",
					3,
					"xapCopyToClipboard"
				],
				[
					"ms-button",
					"",
					3,
					"click",
					"xapCopyToClipboard"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 7)(1, "div", 8), _.Mh(2, 0), _.H(), _.I(3, "button", 9), _.H(), _.F(4, "mat-dialog-content", 10), _.B(5, fBe, 2, 1, "div", 11)(6, lBe, 4, 4), _.H(), _.B(7, nBe, 4, 2, "mat-dialog-actions");
				}
				if (a & 2) {
					_.y(3), _.E("iconName", b.S.ac), _.y(2), _.C(b.tla() ? 5 : 6), _.y(2), _.C(b.tla() ? -1 : 7);
				}
			},
			dependencies: [
				_.Yy,
				_.xC,
				_.sC,
				_.wC,
				_.vC,
				_.zC,
				_.yC,
				_.TC
			],
			styles: [".header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;padding:16px;margin-top:8px}.title[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:16px;font-weight:600;line-height:24px}.value-wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between}.section-wrapper[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;margin-bottom:12px}.details-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px}.right-side[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;gap:12px}.field-header[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;max-width:100%;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text)}.field-value[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-var)}.spinner-container[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:16px}"]
		});
		var kCe = new _.$y("45765657", false);
		var nCe = class {
			constructor() {
				this.S = _.Dk;
				this.ve = {
					wta: 278932,
					xta: 278935,
					yta: 278936,
					bpb: 278931
				};
				this.Za = _.m(_.Iy);
				this.F = _.m(_.Qu);
				this.dialog = _.m(_.rC);
				this.Ksa = _.m(_.Ou);
				this.H = _.m(_.ZC);
				this.route = _.m(_.ll);
				this.A = _.m(_.Op);
				this.I = this.A.getFlag(kCe);
				this.gS = _.M(false);
				this.tv = this.H.A.small;
				this.Jq = _.M(false);
				this.Hb = _.Nn(this.F.Oe);
				this.Nxa = _.W(() => {
					var a;
					var b = ((a = this.Za.ik()) != null ? a : []).length;
					a = this.Za.fa();
					var c = this.gS();
					return this.I && _.Kjd() ? "CREATION_ENABLED" : b >= 50 ? "MAX_IMPORT_PROJECTS_REACHED" : a >= 10 ? "MAX_CREATE_PROJECTS_REACHED" : c ? "CREATING_PROJECT" : "CREATION_ENABLED";
				});
				this.Szb = _.W(() => lCe[this.Nxa()]);
				this.route.queryParams.subscribe((a) => {
					if (a.openImportProjectsPanel === "true") {
						this.vr(true);
					}
				});
			}
			vr(a) {
				if (a) {
					_.Rn(this.Ksa, "API", "Clicked Import Projects Button");
				}
				this.Jq.set(a);
			}
			ZE() {
				var a = this;
				return _.x(function* () {
					a.gS.set(true);
					_.Rn(a.Ksa, "API", "Clicked Create Project Button in projects page");
					yield _.jC(a.dialog.open(_.tE));
					a.gS.set(false);
				});
			}
		};
		nCe.J = function(a) {
			return new (a || nCe)();
		};
		nCe.ka = _.u({
			type: nCe,
			da: [["ms-project-header"]],
			ha: 12,
			ia: 15,
			la: () => [
				"Projects",
				"�*6:1� Understanding projects �/*6:1�",
				" Import projects ",
				"�*2:1� Create a new project in GCP �/*2:1�",
				"�*2:1� Create a new project �/*2:1�",
				[1, "header-container"],
				[1, "header"],
				[1, "right-side"],
				[
					"ms-button",
					"",
					"href",
					"https://ai.google.dev/gemini-api/docs/api-key#google-cloud-projects",
					"target",
					"_blank",
					"data-test-id",
					"understanding-projects-button",
					"aria-label",
					"Understanding projects",
					1,
					"custom-link",
					3,
					"click",
					"variant",
					"iconName",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"href",
					"http://go/aistudio-apikey#create-gcp-project",
					"target",
					"_blank",
					"matTooltip",
					"Google Internal Only",
					"matTooltipPosition",
					"above",
					"aria-label",
					"Create a new project in GCP",
					1,
					"custom-link",
					3,
					"iconName",
					"variant",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"variant",
					"borderless",
					"data-test-id",
					"create-project-button",
					"aria-label",
					"Create a new project",
					3,
					"iconName",
					"disabled",
					"matTooltip",
					"matTooltipDisabled",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"data-test-id",
					"import-projects-button",
					3,
					"click",
					"iconName",
					"disabled",
					"matTooltip",
					"matTooltipDisabled",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					3,
					"onClose",
					"isRightPanelOpen"
				],
				[
					"ms-button",
					"",
					"size",
					"large",
					"variant",
					"borderless",
					"data-test-id",
					"create-project-button",
					"aria-label",
					"Create a new project",
					3,
					"click",
					"iconName",
					"disabled",
					"matTooltip",
					"matTooltipDisabled",
					"ve",
					"veClick",
					"veImpression"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 5)(1, "div", 6), _.Mh(2, 0), _.H(), _.F(3, "div", 7)(4, "a", 8), _.J("click", function() {
						_.Rn(b.Ksa, "API", "Clicked Understanding Projects Button");
					}), _.Kh(5, 1), _.B(6, oBe, 1, 0), _.Lh(), _.H(), _.B(7, qBe, 3, 6, "a", 9)(8, sBe, 3, 8, "button", 10), _.F(9, "button", 11), _.J("click", function() {
						return b.vr(true);
					}), _.Mh(10, 2), _.H()()(), _.F(11, "ms-import-projects-panel", 12), _.J("onClose", function() {
						return b.vr(false);
					}), _.H();
				}
				if (a & 2) {
					_.y(4), _.E("variant", b.tv() ? "icon-borderless" : "borderless")("iconName", b.S.DOCS)("ve", b.ve.bpb)("veClick", true)("veImpression", true), _.y(2), _.C(b.tv() ? -1 : 6), _.y(), _.C(b.Hb ? 7 : 8), _.y(2), _.E("iconName", b.tv() ? undefined : b.S.eib)("disabled", mCe(b))("matTooltip", "You have exceeded the limit of 50 projects that can be imported.")("matTooltipDisabled", !mCe(b))("ve", b.ve.yta)("veClick", true)("veImpression", true), _.y(2), _.E("isRightPanelOpen", b.Jq());
				}
			},
			dependencies: [
				_.Yy,
				_.rE,
				_.IC,
				_.HC,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{width:100%;margin-bottom:20px}.header[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:24px;font-weight:600;line-height:32px}.header-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;gap:8px;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between}@media screen and (max-width:768px){[_nghost-%COMP%]{-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start}[_nghost-%COMP%] > .header-container[_ngcontent-%COMP%]{-webkit-box-ordinal-group:1;-webkit-order:0;-moz-box-ordinal-group:1;-ms-flex-order:0;order:0;-webkit-align-self:flex-start;-ms-flex-item-align:start;align-self:flex-start;width:100%}}.right-side[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-flow:row wrap;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-flow:row wrap;flex-flow:row wrap;gap:12px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}@media screen and (max-width:600px){.right-side[_ngcontent-%COMP%]{gap:2px;-webkit-flex-flow:nowrap;-ms-flex-flow:nowrap;flex-flow:nowrap}}.custom-link[_ngcontent-%COMP%]{color:var(--color-v3-text)}"]
		});
		var oCe = class {
			constructor() {
				this.Za = _.m(_.Iy);
				this.F = _.m(_.Ou);
				this.A = _.m(_.iC);
				this.H = _.m(_.qC);
				this.Jy = _.M("NOT_STARTED");
			}
			removeProject() {
				var a = this;
				return _.x(function* () {
					a.Jy.set("IN_PROGRESS");
					try {
						_.Rn(a.F, "API", "Clicked Remove Project Button in remove dialog");
						yield jCe(a.Za, [a.H.project.getName()]);
						a.A.success("Project removed successfully!");
						a.Jy.set("COMPLETE");
					} catch (b) {
						console.error("Failed to remove project:", b);
						a.A.error("Failed to remove project.");
						a.Jy.set("FAILED");
					}
				});
			}
		};
		oCe.J = function(a) {
			return new (a || oCe)();
		};
		oCe.ka = _.u({
			type: oCe,
			da: [["ms-project-remove-confirmation-dialog"]],
			ha: 3,
			ia: 1,
			la: () => [" The project will be removed from Google AI Studio, but will remain in Google Cloud. You can add it back by importing it into Google AI Studio. ", [
				"dialogTitle",
				"Remove project?",
				"toastWhenDeleting",
				"Removing project...",
				"toastWhenFailed",
				"Failed to remove project. Please retry.",
				"deletingPendingText",
				"Removing...",
				"buttonLabel",
				"Remove",
				3,
				"deletionConfirmed",
				"deletionStatus"
			]],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "ms-delete-confirmation-dialog", 1), _.J("deletionConfirmed", function() {
						return b.removeProject();
					}), _.F(1, "span"), _.Mh(2, 0), _.H()();
				}
				if (a & 2) {
					_.E("deletionStatus", b.Jy());
				}
			},
			dependencies: [_.AC],
			styles: ["[_nghost-%COMP%]{display:block;max-width:min(500px,80vw)}"]
		});
		var pCe = class {
			constructor() {
				this.S = _.Dk;
				this.f7 = _.V(false);
				this.query = _.Mi("");
			}
			oda(a) {
				this.query.set((a != null ? a : "").toString());
			}
		};
		pCe.J = function(a) {
			return new (a || pCe)();
		};
		pCe.ka = _.u({
			type: pCe,
			da: [["ms-project-search-input"]],
			inputs: {
				f7: [1, "focusOnInit"],
				query: [1, "query"]
			},
			outputs: { query: "queryChange" },
			ha: 2,
			ia: 3,
			la: () => [[1, "search-container"], [
				"label",
				"Search for a project",
				"id",
				"project-search-input",
				"placeholder",
				"Search for a project",
				"hideLabel",
				"",
				3,
				"valueChange",
				"icon",
				"value",
				"focusOnInit"
			]],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 0)(1, "ms-input-field", 1), _.J("valueChange", function(c) {
						return b.oda(c);
					}), _.H()();
				}
				if (a & 2) {
					_.y(), _.E("icon", b.S.Lm)("value", b.query())("focusOnInit", b.f7());
				}
			},
			dependencies: [_.mE, _.fF],
			styles: [".search-container[_ngcontent-%COMP%]{max-width:35%}@media screen and (max-width:600px){.search-container[_ngcontent-%COMP%]{max-width:100%}}"]
		});
		var qCe = class {
			L1() {
				var a = this;
				return _.x(function* () {
					var b = a.A();
					if (b) {
						a.yDa.set(true);
						try {
							yield a.Za.L1(_.UQa(b.clone(), a.displayName()));
							a.Wa.close();
							a.F.success("Project renamed successfully!");
						} catch (c) {
							a.F.error("Failed to rename project.");
						} finally {
							a.yDa.set(false);
						}
					}
				});
			}
			OFa(a) {
				if (typeof a === "string") {
					this.displayName.set(a), this.PT.set(_.OAa(a)), this.D3a.set(!!this.PT());
				} else {
					this.displayName.set(""), this.PT.set("");
				}
			}
			constructor() {
				this.S = _.Dk;
				this.Za = _.m(_.Iy);
				this.Wa = _.m(_.kC);
				this.H = _.m(_.qC);
				this.F = _.m(_.iC);
				this.A = _.M();
				this.yDa = _.M(false);
				this.displayName = _.M("");
				this.D3a = _.M(true);
				this.PT = _.M("");
				this.A.set(this.H.project);
				var a;
				var b;
				this.displayName.set((b = (a = this.A()) == null ? undefined : a.getDisplayName()) != null ? b : "");
			}
		};
		qCe.J = function(a) {
			return new (a || qCe)();
		};
		qCe.ka = _.u({
			type: qCe,
			da: [["ms-project-update-dialog"]],
			ha: 6,
			ia: 2,
			la: () => [
				"Rename project",
				" Cancel ",
				" Update ",
				[1, "header"],
				[1, "title"],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"matDialogClose",
					"",
					"aria-label",
					"close",
					3,
					"iconName"
				],
				[1, "spinner-container"],
				[3, "diameter"],
				[1, "display-name-container"],
				[
					"label",
					"Name your project",
					"id",
					"display-name-input",
					"cdkFocusInitial",
					"",
					3,
					"valueChange",
					"value",
					"errorMessage"
				],
				"variant borderless ms-button  matDialogClose ".split(" "),
				[
					"ms-button",
					"",
					"data-test-id",
					"update-button",
					3,
					"click",
					"disabled"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 3)(1, "div", 4), _.Mh(2, 0), _.H(), _.I(3, "button", 5), _.H(), _.B(4, tBe, 3, 1, "mat-dialog-content")(5, uBe, 8, 3);
				}
				if (a & 2) {
					_.y(3), _.E("iconName", b.S.ac), _.y(), _.C(b.yDa() ? 4 : 5);
				}
			},
			dependencies: [
				_.Yy,
				_.JD,
				_.mE,
				_.xC,
				_.sC,
				_.wC,
				_.vC,
				_.$D,
				_.zC,
				_.yC,
				_.IC,
				_.MD
			],
			styles: [".header[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;padding:8px}mat-dialog-content[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px;padding:8px;width:100%}.title[_ngcontent-%COMP%]{margin-left:8px;font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:16px;font-weight:600;line-height:24px}.description[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px}.spinner-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:16px}"]
		});
		var sCe = class {
			constructor() {
				this.ve = {
					Wob: 278945,
					Xob: 278944,
					Yob: 278943,
					Zob: 312770,
					apb: 323759,
					cpb: 278941,
					dpb: 278942
				};
				this.nNb = {
					Y9a: 279808,
					uTa: 297229
				};
				this.jy = rCe;
				this.S = _.Dk;
				this.yhb = {
					message: "You've reached the billing account monthly spend cap and service has been paused.",
					link: "/gemini-api/docs/billing#tier-spend-caps"
				};
				this.Hnb = {
					message: "To purchase credits, go to the Billing page and select Buy credits.",
					link: "/gemini-api/docs/billing#buy-credits"
				};
				this.qob = {
					message: "Your project is required to use a Prepay billing plan.",
					link: "/gemini-api/docs/billing#billing-plans"
				};
				this.sort = _.Ni(_.TI);
				this.OV = _.m(_.Ou);
				this.F = _.m(_.Uy);
				this.A = _.m(_.GG);
				this.qea = _.m(_.Cl);
				this.dialog = _.m(_.rC);
				this.Pm = _.W(() => this.F.A());
				this.fEa = this.F.ef;
				this.c_ = this.A.ma;
				this.headers = _.Li([]);
				this.projects = _.Li([]);
				this.GBa = _.V("");
				this.filter = _.M("");
				this.Wsa = "Can't find your projects here?";
				this.Vsa = "Only imported projects appear here. If you don't see your projects, you can import projects from Google Cloud on this page.";
				this.Gs = _.W(() => {
					var a = this.projects();
					var b = this.filter();
					var c = this.sort();
					a = new _.fJ(a);
					a.F = this.I;
					a.ea = this.H;
					if (c) {
						a.sort = c;
					}
					a.filter = b.trim().toLowerCase();
					return a;
				});
				_.Sy(this.F);
				_.FG(this.A, { fR: true });
			}
			ib() {
				var a = this.GBa();
				this.filter.set(a);
			}
			U$(a) {
				var b = this;
				return _.x(function* () {
					_.Rn(b.OV, "API", "Clicked Create API Key Button");
					var c = {
						II: true,
						fH: a.Ya()
					};
					yield _.pf(_.jC(b.dialog.open(_.AE, { data: c })));
				});
			}
			kna(a) {
				var b = { queryParams: {} };
				var c;
				if (a = (c = _.au(a)) == null ? undefined : c.gk()) {
					b.queryParams.billing = _.Pn(a);
				}
				this.qea.navigate(["billing"], b);
			}
			MGa(a) {
				_.Rn(this.OV, "API", "Clicked View usage data button on a Project");
				this.qea.navigate(["usage"], { queryParams: { project: a.Ya() } });
			}
			I(a, b) {
				switch (b) {
					case "Created On":
						let c, d;
						return (d = (c = a.aj()) == null ? undefined : c.toDate().getTime()) != null ? d : new Date(0).getTime();
					default: return 0;
				}
			}
			L1(a) {
				var b = this;
				return _.x(function* () {
					_.Rn(b.OV, "API", "Clicked Update Project Button");
					b.dialog.open(qCe, { data: { project: a } });
				});
			}
			removeProject(a) {
				var b = this;
				return _.x(function* () {
					_.Rn(b.OV, "API", "Clicked Remove Project Button in projects page");
					yield _.pf(_.jC(b.dialog.open(oCe, {
						data: { project: a },
						id: "delete-project-dialog"
					})));
				});
			}
			H(a, b) {
				return b ? a.getDisplayName().toLowerCase().includes(b) || a.Ya().toLowerCase().includes(b) : true;
			}
			QLa() {
				_.Rn(this.OV, "API", "Clicked Copy Project Id Button");
			}
		};
		sCe.J = function(a) {
			return new (a || sCe)();
		};
		sCe.ka = _.u({
			type: sCe,
			da: [["ms-project-table"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.sort, _.TI, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			inputs: {
				headers: [1, "headers"],
				projects: [1, "projects"],
				GBa: [1, "initialFilter"]
			},
			ha: 32,
			ia: 12,
			la: () => [
				["billingAccountCapReachedTooltipTemplate", ""],
				["noAvailableCreditsTooltipTemplate", ""],
				["prepayRequiredTooltipTemplate", ""],
				["statusTooltipTemplate", ""],
				["overflowmenu", ""],
				"Project",
				"Keys",
				" Create API key ",
				"Created",
				"Billing Tier",
				"Status",
				" Action needed ",
				" Buy credits ",
				" No credits ",
				" Prepay required ",
				" BA cap hit ",
				" Rename project ",
				" Remove project ",
				" Learn more ",
				[
					3,
					"queryChange",
					"query",
					"focusOnInit"
				],
				[1, "project-table-container"],
				[
					"mat-table",
					"",
					"matSort",
					"",
					1,
					"mat-elevation-z8",
					3,
					"dataSource"
				],
				[3, "matColumnDef"],
				[
					"mat-header-cell",
					"",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell",
					4,
					"matCellDef"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"",
					3,
					"ve",
					"veClick",
					"veImpression",
					4,
					"matHeaderCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell created-on",
					4,
					"matCellDef"
				],
				[
					"mat-cell",
					"",
					"class",
					"table-body-cell usage-count",
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
					"learnMoreUrl",
					"https://ai.google.dev/gemini-api/docs/api-key#import-projects",
					3,
					"headline",
					"message",
					"showSparkle",
					"showImportProjectsButton",
					"showCreateProjectButton"
				],
				["mat-header-cell", ""],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell"
				],
				[1, "project"],
				[
					"ms-button",
					"",
					"variant",
					"link",
					1,
					"project-table-link",
					3,
					"click"
				],
				[1, "sub-text"],
				["mode", "indeterminate"],
				[
					"ms-button",
					"",
					"variant",
					"link",
					1,
					"project-table-link",
					"api-key-create-button"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"View API keys",
					1,
					"project-table-link",
					"api-key-count-button"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					1,
					"project-table-link",
					"api-key-create-button",
					3,
					"click"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"View API keys",
					1,
					"project-table-link",
					"api-key-count-button",
					3,
					"click"
				],
				[
					"mat-header-cell",
					"",
					"mat-sort-header",
					"",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell",
					"created-on"
				],
				[
					"mat-cell",
					"",
					1,
					"table-body-cell",
					"usage-count"
				],
				[
					3,
					"project",
					"veMap"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"Payment action needed",
					1,
					"project-table-link",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"Buy credits",
					1,
					"project-table-link",
					3,
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"dialogLabel",
					"No available credits information",
					1,
					"badge",
					"alert",
					3,
					"xapInlineDialog"
				],
				[
					"dialogLabel",
					"Prepay required information",
					1,
					"badge",
					"alert",
					3,
					"xapInlineDialog"
				],
				[
					"dialogLabel",
					"Billing account cap hit information",
					1,
					"badge",
					"alert",
					3,
					"xapInlineDialog"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"Payment action needed",
					1,
					"project-table-link",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"variant",
					"link",
					"aria-label",
					"Buy credits",
					1,
					"project-table-link",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				],
				[1, "actions"],
				[
					"matTooltip",
					"Copy project ID",
					"matTooltipPosition",
					"below"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"Copy project ID",
					3,
					"click",
					"iconName",
					"xapCopyToClipboard"
				],
				[
					"matTooltip",
					"View spend",
					"matTooltipPosition",
					"below"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"View spend",
					3,
					"click",
					"iconName",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"matTooltip",
					"View usage",
					"matTooltipPosition",
					"below"
				],
				[
					"ms-button",
					"",
					"variant",
					"icon-borderless",
					"aria-label",
					"View usage",
					3,
					"click",
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
					"aria-label",
					"View more actions",
					"matTooltip",
					"View more actions",
					3,
					"matMenuTriggerFor",
					"iconName"
				],
				[
					"mat-menu-item",
					"",
					"variant",
					"borderless",
					"data-test-rename-project",
					"",
					"matTooltipPosition",
					"left",
					3,
					"click",
					"matTooltip",
					"matTooltipDisabled",
					"disabled",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					1,
					"start-icon",
					3,
					"iconName"
				],
				[
					"mat-menu-item",
					"",
					"variant",
					"borderless",
					"data-test-remove-project",
					"",
					3,
					"click",
					"ve",
					"veClick",
					"veImpression"
				],
				["mat-header-row", ""],
				["mat-row", ""],
				[
					4,
					"ngTemplateOutlet",
					"ngTemplateOutletContext"
				],
				[1, "status-tooltip"],
				[
					"data-test-id",
					"status-learn-more-link",
					3,
					"documentation-path"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "ms-project-search-input", 19), _.J("queryChange", function(c) {
						b.filter.set(c);
					}), _.H(), _.F(1, "div", 20)(2, "table", 21), _.Gh(3, 22), _.z(4, vBe, 2, 0, "th", 23)(5, xBe, 6, 2, "td", 24), _.Hh(), _.Gh(6, 22), _.z(7, yBe, 2, 0, "th", 23)(8, EBe, 4, 1, "td", 24), _.Hh(), _.Gh(9, 22), _.z(10, FBe, 2, 3, "th", 25)(11, GBe, 3, 3, "td", 26), _.Hh(), _.Gh(12, 22), _.z(13, HBe, 2, 0, "th", 23)(14, KBe, 3, 1, "td", 27), _.Hh(), _.Gh(15, 22), _.z(16, LBe, 2, 0, "th", 23)(17, TBe, 6, 5, "td", 24), _.Hh(), _.Gh(18, 22), _.z(19, UBe, 1, 0, "th", 23)(20, VBe, 19, 23, "td", 24), _.Hh(), _.z(21, WBe, 1, 0, "tr", 28)(22, XBe, 1, 0, "tr", 29), _.H()(), _.B(23, YBe, 1, 5, "ms-project-zero-state", 30), _.z(24, aCe, 1, 4, "ng-template", null, 0, _.Ii)(26, cCe, 1, 4, "ng-template", null, 1, _.Ii)(28, eCe, 1, 4, "ng-template", null, 2, _.Ii)(30, fCe, 5, 2, "ng-template", null, 3, _.Ii);
				}
				if (a & 2) {
					_.E("query", b.filter())("focusOnInit", !!b.GBa()), _.y(2), _.E("dataSource", b.Gs()), _.y(), _.E("matColumnDef", b.jy.mta), _.y(3), _.E("matColumnDef", b.jy.Xlb), _.y(3), _.E("matColumnDef", b.jy.Tra), _.y(3), _.E("matColumnDef", b.jy.zta), _.y(3), _.E("matColumnDef", b.jy.Kta), _.y(3), _.E("matColumnDef", b.jy.xra), _.y(3), _.E("matHeaderRowDef", b.headers()), _.y(), _.E("matRowDefColumns", b.headers()), _.y(), _.C(b.Gs().Nia.length <= 1 ? 23 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.LC,
				_.dz,
				_.wI,
				_.tI,
				_.sI,
				_.vI,
				_.tO,
				_.sO,
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
				_.HC,
				_.nz,
				pCe,
				_.n3,
				_.l3,
				_.Bz,
				_.TC,
				_.EC,
				_.pz
			],
			styles: ["[_nghost-%COMP%]   .mat-mdc-row[_ngcontent-%COMP%]   .mat-mdc-cell[_ngcontent-%COMP%]{padding-top:16px;padding-bottom:16px}[_nghost-%COMP%]   .mat-mdc-header-row[_ngcontent-%COMP%]{height:48px}.badge[_ngcontent-%COMP%]{border-radius:8px;padding:1px 6px 1px 5px;border:1px solid var(--color-v3-outline);background-color:var(--color-v3-surface-container-high);color:var(--color-v3-text);display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:5px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px}.badge[_ngcontent-%COMP%]:before{content:\"\";width:6px;aspect-ratio:1/1;border-radius:50%;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.badge.enabled[_ngcontent-%COMP%]:before, .badge.green[_ngcontent-%COMP%]:before, .badge.new[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-4)}.badge.gray[_ngcontent-%COMP%]:before, .badge.not-enabled[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-var)}.badge.confidential[_ngcontent-%COMP%]:before, .badge.orange[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-1)}.badge.blue[_ngcontent-%COMP%]:before, .badge.paid[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-link)}.badge.alert[_ngcontent-%COMP%]:before, .badge.red[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-3)}.badge.hide-circle[_ngcontent-%COMP%]:before{display:none}.project-table-container[_ngcontent-%COMP%]{overflow-x:auto;margin-top:20px}.table-body-cell[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;vertical-align:baseline}.project[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:start;-moz-box-align:start;-ms-flex-align:start;align-items:start;gap:2px}.project-table-link[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;padding:0;margin:0;border:none;height:24px}.sub-text[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var)}.actions[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text-var);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;gap:8px;-webkit-box-pack:end;-webkit-justify-content:end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:end}.mat-column-Quota-Tier[_ngcontent-%COMP%]{width:12%;min-width:118px}.mat-column-Status[_ngcontent-%COMP%]{width:12%;min-width:100px;max-width:200px}.mat-column-Status[_ngcontent-%COMP%]:has(.badge){width:15%;min-width:170px}.mat-column-Actions[_ngcontent-%COMP%], .mat-column-Keys[_ngcontent-%COMP%]{width:12%}.mat-column-Actions[_ngcontent-%COMP%], .mat-column-Keys[_ngcontent-%COMP%]:has(mat-progress-bar), .mat-column-Quota-Tier[_ngcontent-%COMP%]:has(mat-progress-bar){vertical-align:middle}mat-progress-bar[_ngcontent-%COMP%]{--mat-progress-bar-active-indicator-height:2px;--mat-progress-bar-track-height:2px;width:30px}.mat-column-Created-On[_ngcontent-%COMP%]{width:15%;min-width:124px}.api-key-count-button[_ngcontent-%COMP%]{padding-left:0}.status-tooltip[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;padding:16px}"]
		});
		_.us = class {
			constructor() {
				this.Za = _.m(_.Iy);
				this.F = _.m(_.sG);
				this.V1a = _.W(() => (this.Za.je() || this.F.R()) && this.Sd().length === 0);
				this.A = _.m(_.$E);
				this.H = this.A.get("projectFilter");
				this.X1a = null;
				this.Sd = this.Za.Sd;
				this.headers = "Project;Keys;Created On;Quota Tier;Status;Actions".split(";");
				_.Gy(this.Za);
			}
			ib() {
				this.X1a = this.H();
				this.A.set({ projectFilter: null });
			}
		};
		_.us.J = function(a) {
			return new (a || _.us)();
		};
		_.us.ka = _.u({
			type: _.us,
			da: [["ms-projects"]],
			ha: 6,
			ia: 6,
			la: [
				[1, "page-content-wrapper"],
				[1, "page-content-inner-wrapper"],
				[1, "loading-spinner"],
				[
					3,
					"headers",
					"projects",
					"initialFilter"
				],
				["diameter", "24"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 0)(1, "div", 1), _.I(2, "ms-project-header")(3, "ms-payment-alert-callout"), _.B(4, gCe, 2, 0, "div", 2), _.I(5, "ms-project-table", 3), _.H()();
				}
				if (a & 2) {
					_.y(4);
					_.C(b.V1a() ? 4 : -1);
					_.y();
					_.P("hidden", b.V1a());
					let c;
					_.E("headers", b.headers)("projects", b.Sd())("initialFilter", (c = b.X1a) != null ? c : "");
				}
			},
			dependencies: [
				_.zC,
				_.yC,
				nCe,
				sCe,
				_.m3
			],
			styles: ["[_nghost-%COMP%]{display:block}[_nghost-%COMP%]   .page-content-inner-wrapper[_ngcontent-%COMP%]{max-width:min(1400px,90%)}@media screen and (max-width:600px){[_nghost-%COMP%]   .page-content-inner-wrapper[_ngcontent-%COMP%]{max-width:100%}}.loading-spinner[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;padding:16px}ms-project-table.hidden[_ngcontent-%COMP%]{display:none}"]
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

