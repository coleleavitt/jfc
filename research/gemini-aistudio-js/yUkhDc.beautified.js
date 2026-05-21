"use strict";
this.default_MakerSuite = this.default_MakerSuite || {};
(function(_) {
	var window = this;
	try {
		_.bXb = function(a, b, c) {
			c = c || _.Haa;
			for (var d = 0, e = a.length, f; d < e;) {
				let g = d + (e - d >>> 1);
				let k;
				k = c(b, a[g]);
				if (k > 0) {
					d = g + 1;
				} else {
					e = g, f = !k;
				}
			}
			return f ? d : -d - 1;
		};
		_.nj.prototype.hasImage = _.ca(13, function() {
			return _.Dr(this, _.wv, 2, _.vj);
		});
		_.Gpa.prototype.hasImage = _.ca(12, function() {
			return _.Dr(this, _.wv, 2, _.wj);
		});
		_.Av.prototype.hasImage = _.ca(11, function() {
			return _.Dr(this, _.wv, 2, _.kj);
		});
		_.Zp.prototype.hasImage = _.ca(10, function() {
			return _.Dr(this, _.w0a, 2, _.EHa);
		});
		_.xx.prototype.hasImage = _.ca(9, function() {
			return _.Dr(this, _.Q2a, 2, _.wx);
		});
		_.zx.prototype.hasImage = _.ca(8, function() {
			return _.Dr(this, _.Q2a, 2, _.yx);
		});
		_.eO = class {};
		_.eO.J = function(a) {
			return new (a || _.eO)();
		};
		_.eO.Oa = _.We({
			type: _.eO,
			da: [["mat-card-content"]],
			eb: [1, "mat-mdc-card-content"]
		});
		_.aP = function() {
			var a = window;
			if (a) return a !== a.parent && a.hasOwnProperty("pangolinIframeGlobals");
			var b = Error("Ef");
			setTimeout(() => {
				throw b;
			}, 1);
			return false;
		};
		_.bP = function(a) {
			if (!a1b) {
				let b;
				let c;
				a1b = (a = (b = a.document) == null ? undefined : (c = b.querySelector("pan-shell")) == null ? undefined : c.dataset.initArgs) ? _.Dea(a, c1b) : new d1b();
			}
			return a1b;
		};
		_.cP = function(a) {
			var b = _.e1b(a);
			if (!f1b.has(a)) {
				f1b.add(a), g1b.add(a);
			}
			return b;
		};
		_.e1b = function(a) {
			var b = _.h1b();
			if (typeof window !== "undefined" && recordAccessedExperimentFlags) {
				recordAccessedExperimentFlags(a);
			}
			return b && b.typescript_experiment_flags ? b.typescript_experiment_flags.getFlag(a) : a.defaultValue;
		};
		j1b = function() {
			i1b.bHa = undefined;
			i1b.p2 = undefined;
		};
		_.h1b = function() {
			return typeof window === "undefined" ? {} : _.aP() ? parent : window;
		};
		_.k1b = [1, 2];
		_.l1b = [1, 2];
		_.dP = [
			2,
			3,
			4,
			5,
			6,
			7
		];
		_.eP = class extends Window {};
		_.eP.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(_.eP)))(b || _.eP);
			};
		})();
		_.eP.sa = _.Cd({
			token: _.eP,
			factory: function(a) {
				var b = null;
				if (a) {
					b = new (a || _.eP)();
				} else {
					b = window;
				}
				return b;
			},
			wa: "root"
		});
		_.fP = class {
			get fI() {
				return (_.aP() ? this.ref.parent : this.ref).document.hasFocus();
			}
			constructor() {
				var a = this.ref = _.m(_.eP);
				this.wh = _.Af(a, "blur");
				this.Yk = _.Af(a, "focus");
				this.qz = _.Af(a, "resize");
			}
			open(a, b = "_blank") {
				_.rd(_.aP() ? this.ref.parent : this.ref, _.kd(a), b, "noopener,noreferrer");
			}
		};
		_.fP.J = function(a) {
			return new (a || _.fP)();
		};
		_.fP.sa = _.Cd({
			token: _.fP,
			factory: _.fP.J,
			wa: "root"
		});
		var d1b = class extends _.h {
			constructor(a) {
				super(a);
			}
			hasRef() {
				return _.zn(this, 8);
			}
		};
		var c1b = _.bd(d1b);
		var a1b = undefined;
		var m1b = class {
			constructor() {
				this.A = {};
			}
			getFlag(a) {
				var b = this.A[a.key];
				if (a.valueType === "proto") {
					{
						let c = JSON.parse(b);
						if (Array.isArray(c)) return c;
					}
					return a.defaultValue;
				}
				return typeof b === typeof a.defaultValue ? b : a.defaultValue;
			}
			parsePayload(a) {
				{
					let b = JSON.parse(a)[0];
					a = "";
					for (let c = 0; c < b.length; c++) a += String.fromCharCode(b.charCodeAt(c) ^ "\x07\x07\b".charCodeAt(c % 10));
					this.A = JSON.parse(a);
				}
			}
		};
		var f1b = new Set();
		var g1b = new Set();
		_.x(function* () {});
		_.gP = {};
		if (window) {
			let a = _.h1b();
			if (!a.typescript_experiment_flags) {
				var n1b;
				var o1b = _.bP(a);
				n1b = _.l(o1b, 22);
				a.typescript_experiment_flags = new m1b();
				a.typescript_experiment_flags.parsePayload(n1b);
			}
			window.invalidateFlagsCache = j1b;
		}
		var i1b = _.gP;
		_.hP = class {
			constructor(a) {
				this.key = a;
				this.defaultValue = false;
				this.valueType = "boolean";
			}
		};
		p1b = new Map([
			["sdui.sys.color.primary", "--sdui-sys-color-primary"],
			["sdui.sys.color.on-primary", "--sdui-sys-color-on-primary"],
			["sdui.sys.color.inverse-primary", "--sdui-sys-color-inverse-primary"],
			["sdui.sys.color.inverse-on-primary", "--sdui-sys-color-inverse-on-primary"],
			["sdui.sys.color.primary-container", "--sdui-sys-color-primary-container"],
			["sdui.sys.color.on-primary-container", "--sdui-sys-color-on-primary-container"],
			["sdui.sys.color.secondary", "--sdui-sys-color-secondary"],
			["sdui.sys.color.on-secondary", "--sdui-sys-color-on-secondary"],
			["sdui.sys.color.secondary-container", "--sdui-sys-color-secondary-container"],
			["sdui.sys.color.on-secondary-container", "--sdui-sys-color-on-secondary-container"],
			["sdui.sys.color.surface", "--sdui-sys-color-surface"],
			["sdui.sys.color.inverse-surface", "--sdui-sys-color-inverse-surface"],
			["sdui.sys.color.on-surface", "--sdui-sys-color-on-surface"],
			["sdui.sys.color.inverse-on-surface", "--sdui-sys-color-inverse-on-surface"],
			["sdui.sys.color.on-surface-variant", "--sdui-sys-color-on-surface-variant"],
			["sdui.sys.color.surface-container-lowest", "--sdui-sys-color-surface-container-lowest"],
			["sdui.sys.color.surface-container-low", "--sdui-sys-color-surface-container-low"],
			["sdui.sys.color.surface-container", "--sdui-sys-color-surface-container"],
			["sdui.sys.color.surface-container-high", "--sdui-sys-color-surface-container-high"],
			["sdui.sys.color.surface-container-highest", "--sdui-sys-color-surface-container-highest"],
			["sdui.sys.color.scrim", "--sdui-sys-color-scrim"],
			["sdui.sys.color.shadow", "--sdui-sys-color-shadow"],
			["sdui.sys.color.outline", "--sdui-sys-color-outline"],
			["sdui.sys.color.outline-variant", "--sdui-sys-color-outline-variant"],
			["sdui.sys.color.disabled", "--sdui-sys-color-disabled"],
			["sdui.sys.color.disabled-container", "--sdui-sys-color-disabled-container"],
			["sdui.sys.color.focus-primary", "--sdui-sys-color-focus-primary"],
			["sdui.sys.color.focus-primary-container", "--sdui-sys-color-focus-primary-container"],
			["sdui.sys.color.focus-secondary", "--sdui-sys-color-focus-secondary"],
			["sdui.sys.color.focus-secondary-container", "--sdui-sys-color-focus-secondary-container"],
			["sdui.sys.color.hover-primary", "--sdui-sys-color-hover-primary"],
			["sdui.sys.color.hover-primary-container", "--sdui-sys-color-hover-primary-container"],
			["sdui.sys.color.hover-secondary", "--sdui-sys-color-hover-secondary"],
			["sdui.sys.color.hover-secondary-container", "--sdui-sys-color-hover-secondary-container"],
			["sdui.sys.color.pressed-primary", "--sdui-sys-color-pressed-primary"],
			["sdui.sys.color.pressed-primary-container", "--sdui-sys-color-pressed-primary-container"],
			["sdui.sys.color.pressed-secondary", "--sdui-sys-color-pressed-secondary"],
			["sdui.sys.color.pressed-secondary-container", "--sdui-sys-color-pressed-secondary-container"],
			["sdui.sys.color.status.success", "--sdui-sys-color-status-success"],
			["sdui.sys.color.status.on-success", "--sdui-sys-color-status-on-success"],
			["sdui.sys.color.status.success-container", "--sdui-sys-color-status-success-container"],
			["sdui.sys.color.status.on-success-container", "--sdui-sys-color-status-on-success-container"],
			["sdui.sys.color.status.warning", "--sdui-sys-color-status-warning"],
			["sdui.sys.color.status.on-warning", "--sdui-sys-color-status-on-warning"],
			["sdui.sys.color.status.warning-container", "--sdui-sys-color-status-warning-container"],
			["sdui.sys.color.status.on-warning-container", "--sdui-sys-color-status-on-warning-container"],
			["sdui.sys.color.status.error", "--sdui-sys-color-status-error"],
			["sdui.sys.color.status.on-error", "--sdui-sys-color-status-on-error"],
			["sdui.sys.color.status.error-container", "--sdui-sys-color-status-error-container"],
			["sdui.sys.color.status.on-error-container", "--sdui-sys-color-status-on-error-container"],
			["sdui.sys.color.status.neutral", "--sdui-sys-color-status-neutral"],
			["sdui.sys.color.status.on-neutral", "--sdui-sys-color-status-on-neutral"],
			["sdui.sys.color.status.neutral-container", "--sdui-sys-color-status-neutral-container"],
			["sdui.sys.color.status.on-neutral-container", "--sdui-sys-color-status-on-neutral-container"],
			["sdui.sys.color.link.default", "--sdui-sys-color-link-default"],
			["sdui.sys.color.link.visited", "--sdui-sys-color-link-visited"],
			["sdui.sys.color.charts.primary-outline", "--sdui-sys-color-charts-primary-outline"],
			["sdui.sys.color.charts.primary-default", "--sdui-sys-color-charts-primary-default"],
			["sdui.sys.color.charts.primary-medium", "--sdui-sys-color-charts-primary-medium"],
			["sdui.sys.color.charts.primary-low", "--sdui-sys-color-charts-primary-low"],
			["sdui.sys.color.charts.neutral-outline", "--sdui-sys-color-charts-neutral-outline"],
			["sdui.sys.color.charts.neutral-default", "--sdui-sys-color-charts-neutral-default"],
			["sdui.sys.color.charts.neutral-medium", "--sdui-sys-color-charts-neutral-medium"],
			["sdui.sys.color.charts.neutral-low", "--sdui-sys-color-charts-neutral-low"],
			["sdui.sys.color.charts.success-outline", "--sdui-sys-color-charts-success-outline"],
			["sdui.sys.color.charts.success-default", "--sdui-sys-color-charts-success-default"],
			["sdui.sys.color.charts.success-medium", "--sdui-sys-color-charts-success-medium"],
			["sdui.sys.color.charts.success-low", "--sdui-sys-color-charts-success-low"],
			["sdui.sys.color.charts.warning-outline", "--sdui-sys-color-charts-warning-outline"],
			["sdui.sys.color.charts.warning-default", "--sdui-sys-color-charts-warning-default"],
			["sdui.sys.color.charts.warning-medium", "--sdui-sys-color-charts-warning-medium"],
			["sdui.sys.color.charts.warning-low", "--sdui-sys-color-charts-warning-low"],
			["sdui.sys.color.charts.error-outline", "--sdui-sys-color-charts-error-outline"],
			["sdui.sys.color.charts.error-default", "--sdui-sys-color-charts-error-default"],
			["sdui.sys.color.charts.error-medium", "--sdui-sys-color-charts-error-medium"],
			["sdui.sys.color.charts.error-low", "--sdui-sys-color-charts-error-low"],
			["sdui.sys.color.charts.category1-outline", "--sdui-sys-color-charts-category1-outline"],
			["sdui.sys.color.charts.category1-default", "--sdui-sys-color-charts-category1-default"],
			["sdui.sys.color.charts.category1-high", "--sdui-sys-color-charts-category1-high"],
			["sdui.sys.color.charts.category1-medium", "--sdui-sys-color-charts-category1-medium"],
			["sdui.sys.color.charts.category1-low", "--sdui-sys-color-charts-category1-low"],
			["sdui.sys.color.charts.category2-outline", "--sdui-sys-color-charts-category2-outline"],
			["sdui.sys.color.charts.category2-default", "--sdui-sys-color-charts-category2-default"],
			["sdui.sys.color.charts.category2-high", "--sdui-sys-color-charts-category2-high"],
			["sdui.sys.color.charts.category2-medium", "--sdui-sys-color-charts-category2-medium"],
			["sdui.sys.color.charts.category2-low", "--sdui-sys-color-charts-category2-low"],
			["sdui.sys.color.charts.category3-outline", "--sdui-sys-color-charts-category3-outline"],
			["sdui.sys.color.charts.category3-default", "--sdui-sys-color-charts-category3-default"],
			["sdui.sys.color.charts.category3-high", "--sdui-sys-color-charts-category3-high"],
			["sdui.sys.color.charts.category3-medium", "--sdui-sys-color-charts-category3-medium"],
			["sdui.sys.color.charts.category3-low", "--sdui-sys-color-charts-category3-low"],
			["sdui.sys.color.charts.category4-outline", "--sdui-sys-color-charts-category4-outline"],
			["sdui.sys.color.charts.category4-default", "--sdui-sys-color-charts-category4-default"],
			["sdui.sys.color.charts.category4-high", "--sdui-sys-color-charts-category4-high"],
			["sdui.sys.color.charts.category4-medium", "--sdui-sys-color-charts-category4-medium"],
			["sdui.sys.color.charts.category4-low", "--sdui-sys-color-charts-category4-low"],
			["sdui.sys.color.charts.category5-outline", "--sdui-sys-color-charts-category5-outline"],
			["sdui.sys.color.charts.category5-default", "--sdui-sys-color-charts-category5-default"],
			["sdui.sys.color.charts.category5-high", "--sdui-sys-color-charts-category5-high"],
			["sdui.sys.color.charts.category5-medium", "--sdui-sys-color-charts-category5-medium"],
			["sdui.sys.color.charts.category5-low", "--sdui-sys-color-charts-category5-low"],
			["sdui.sys.color.charts.category6-outline", "--sdui-sys-color-charts-category6-outline"],
			["sdui.sys.color.charts.category6-default", "--sdui-sys-color-charts-category6-default"],
			["sdui.sys.color.charts.category6-high", "--sdui-sys-color-charts-category6-high"],
			["sdui.sys.color.charts.category6-medium", "--sdui-sys-color-charts-category6-medium"],
			["sdui.sys.color.charts.category6-low", "--sdui-sys-color-charts-category6-low"],
			["sdui.sys.color.charts.category7-outline", "--sdui-sys-color-charts-category7-outline"],
			["sdui.sys.color.charts.category7-default", "--sdui-sys-color-charts-category7-default"],
			["sdui.sys.color.charts.category7-high", "--sdui-sys-color-charts-category7-high"],
			["sdui.sys.color.charts.category7-medium", "--sdui-sys-color-charts-category7-medium"],
			["sdui.sys.color.charts.category7-low", "--sdui-sys-color-charts-category7-low"],
			["sdui.sys.color.charts.category8-outline", "--sdui-sys-color-charts-category8-outline"],
			["sdui.sys.color.charts.category8-default", "--sdui-sys-color-charts-category8-default"],
			["sdui.sys.color.charts.category8-high", "--sdui-sys-color-charts-category8-high"],
			["sdui.sys.color.charts.category8-medium", "--sdui-sys-color-charts-category8-medium"],
			["sdui.sys.color.charts.category8-low", "--sdui-sys-color-charts-category8-low"],
			["sdui.sys.color.charts.area.category1-outline", "--sdui-sys-color-charts-area-category1-outline"],
			["sdui.sys.color.charts.area.category1-fill", "--sdui-sys-color-charts-area-category1-fill"],
			["sdui.sys.color.charts.area.category2-outline", "--sdui-sys-color-charts-area-category2-outline"],
			["sdui.sys.color.charts.area.category2-fill", "--sdui-sys-color-charts-area-category2-fill"],
			["sdui.sys.color.charts.area.category3-outline", "--sdui-sys-color-charts-area-category3-outline"],
			["sdui.sys.color.charts.area.category3-fill", "--sdui-sys-color-charts-area-category3-fill"],
			["sdui.sys.color.charts.area.category4-outline", "--sdui-sys-color-charts-area-category4-outline"],
			["sdui.sys.color.charts.area.category4-fill", "--sdui-sys-color-charts-area-category4-fill"],
			["sdui.sys.color.charts.area.category5-outline", "--sdui-sys-color-charts-area-category5-outline"],
			["sdui.sys.color.charts.area.category5-fill", "--sdui-sys-color-charts-area-category5-fill"],
			["sdui.sys.color.charts.area.category6-outline", "--sdui-sys-color-charts-area-category6-outline"],
			["sdui.sys.color.charts.area.category6-fill", "--sdui-sys-color-charts-area-category6-fill"],
			["sdui.sys.color.charts.area.category7-outline", "--sdui-sys-color-charts-area-category7-outline"],
			["sdui.sys.color.charts.area.category7-fill", "--sdui-sys-color-charts-area-category7-fill"],
			["sdui.sys.color.charts.area.category8-outline", "--sdui-sys-color-charts-area-category8-outline"],
			["sdui.sys.color.charts.area.category8-fill", "--sdui-sys-color-charts-area-category8-fill"],
			["sdui.sys.color.charts.line.category1", "--sdui-sys-color-charts-line-category1"],
			["sdui.sys.color.charts.line.category2", "--sdui-sys-color-charts-line-category2"],
			["sdui.sys.color.charts.line.category3", "--sdui-sys-color-charts-line-category3"],
			["sdui.sys.color.charts.line.category4", "--sdui-sys-color-charts-line-category4"],
			["sdui.sys.color.charts.line.category5", "--sdui-sys-color-charts-line-category5"],
			["sdui.sys.color.charts.line.category6", "--sdui-sys-color-charts-line-category6"],
			["sdui.sys.color.charts.line.category7", "--sdui-sys-color-charts-line-category7"],
			["sdui.sys.color.charts.line.category8", "--sdui-sys-color-charts-line-category8"],
			["sdui.sys.color.charts.line.category9", "--sdui-sys-color-charts-line-category9"],
			["sdui.sys.color.charts.line.category10", "--sdui-sys-color-charts-line-category10"],
			["sdui.sys.color.charts.line.category11", "--sdui-sys-color-charts-line-category11"],
			["sdui.sys.color.charts.line.category12", "--sdui-sys-color-charts-line-category12"],
			["sdui.sys.color.charts.axis", "--sdui-sys-color-charts-axis"],
			["sdui.sys.color.charts.axis-label", "--sdui-sys-color-charts-axis-label"],
			["sdui.sys.color.charts.legend-label", "--sdui-sys-color-charts-legend-label"],
			["sdui.sys.color.charts.ticks", "--sdui-sys-color-charts-ticks"],
			["sdui.sys.color.charts.grid", "--sdui-sys-color-charts-grid"],
			["sdui.sys.color.charts.threshold", "--sdui-sys-color-charts-threshold"],
			["sdui.sys.color.charts.unfilled", "--sdui-sys-color-charts-unfilled"],
			["sdui.sys.color.charts.card-surface", "--sdui-sys-color-charts-card-surface"],
			["sdui.sys.color.charts.title", "--sdui-sys-color-charts-title"],
			["sdui.sys.color.charts.subtitle", "--sdui-sys-color-charts-subtitle"],
			["sdui.ref.space.0", "--sdui-ref-space-0"],
			["sdui.ref.space.05", "--sdui-ref-space-05"],
			["sdui.ref.space.1", "--sdui-ref-space-1"],
			["sdui.ref.space.2", "--sdui-ref-space-2"],
			["sdui.ref.space.3", "--sdui-ref-space-3"],
			["sdui.ref.space.4", "--sdui-ref-space-4"],
			["sdui.ref.space.5", "--sdui-ref-space-5"],
			["sdui.ref.space.6", "--sdui-ref-space-6"],
			["sdui.ref.space.7", "--sdui-ref-space-7"],
			["sdui.ref.space.8", "--sdui-ref-space-8"],
			["sdui.ref.space.9", "--sdui-ref-space-9"],
			["sdui.ref.space.10", "--sdui-ref-space-10"],
			["sdui.ref.space.12", "--sdui-ref-space-12"],
			["sdui.ref.space.14", "--sdui-ref-space-14"],
			["sdui.sys.type.display-small", "--sdui-sys-type-display-small"],
			["sdui.sys.type.display-medium", "--sdui-sys-type-display-medium"],
			["sdui.sys.type.display-large", "--sdui-sys-type-display-large"],
			["sdui.sys.type.headline-small", "--sdui-sys-type-headline-small"],
			["sdui.sys.type.headline-medium", "--sdui-sys-type-headline-medium"],
			["sdui.sys.type.headline-large", "--sdui-sys-type-headline-large"],
			["sdui.sys.type.title-small", "--sdui-sys-type-title-small"],
			["sdui.sys.type.title-medium", "--sdui-sys-type-title-medium"],
			["sdui.sys.type.title-large", "--sdui-sys-type-title-large"],
			["sdui.sys.type.label-small", "--sdui-sys-type-label-small"],
			["sdui.sys.type.label-medium", "--sdui-sys-type-label-medium"],
			["sdui.sys.type.label-large", "--sdui-sys-type-label-large"],
			["sdui.sys.type.body-small", "--sdui-sys-type-body-small"],
			["sdui.sys.type.body-medium", "--sdui-sys-type-body-medium"],
			["sdui.sys.type.body-large", "--sdui-sys-type-body-large"],
			["sdui.sys.type.code", "--sdui-sys-type-code"],
			["sdui.ref.type.family.display", "--sdui-ref-type-family-display"],
			["sdui.ref.type.family.body", "--sdui-ref-type-family-body"],
			["sdui.ref.type.family.code", "--sdui-ref-type-family-code"]
		]);
		_.iP = function(a, b) {
			if (a.A.has(b)) return a.A.get(b);
			if (!b.startsWith("sdui")) return b;
			if (p1b.has(b)) {
				var c = `var(${p1b.get(b)})`;
				a.A.set(b, c);
				return c;
			}
		};
		_.jP = class {
			constructor() {
				this.A = new Map();
			}
		};
		_.kP = new _.he("SduiComponentPrimitive");
		_.lP = function(a) {
			var b = new _.Zg(a);
			return (c) => _.Pla(() => b)(c);
		};
		_.mP = function(a) {
			return document.createRange().createContextualFragment(_.qd(_.pd(a[0])));
		};
		var q1b;
		var r1b;
		var t1b;
		var x1b;
		var nP;
		var A1b;
		var B1b;
		var C1b;
		var D1b;
		var F1b;
		var H1b;
		var J1b;
		var M1b;
		var U1b;
		var V1b;
		var W1b;
		var b2b;
		var g2b;
		var h2b;
		var i2b;
		var j2b;
		var k2b;
		var l2b;
		q1b = function(a, b) {
			a = a.split("%s");
			var c = "";
			var d = a.length - 1;
			for (let e = 0; e < d; e++) c += a[e] + (e < b.length ? b[e] : "%s");
			_.da.call(this, c + a[d]);
		};
		r1b = function(a) {
			return (b) => {
				var c = [];
				for (let d = 0; c && !b.closed && d < a.length; d++) c.push(_.gf(a[d]).subscribe(new _.sf(b, (e) => {
					if (c) {
						for (let f = 0; f < c.length; f++) f !== d && c[f].unsubscribe();
						c = null;
					}
					b.next(e);
				})));
			};
		};
		_.s1b = function(...a) {
			a = _.Xha(a);
			return a.length === 1 ? _.gf(a[0]) : new _.ef(r1b(a));
		};
		_.u1b = function() {
			var a = t1b();
			a = _.bP(a);
			return _.Pm(a, 24);
		};
		t1b = function() {
			return typeof window === "undefined" ? {} : _.aP() ? parent : window;
		};
		v1b = function() {
			var a = document;
			var b = new _.Wg();
			a.addEventListener("visibilitychange", () => {
				if (a.hidden) {
					b.next();
				}
			});
			return b;
		};
		w1b = function(a, b) {
			if ((a = a.get("x-debug-tracking-id")) && (a = a.match(/([\d]+)(?:;\w=([01]))?/)) && !(a.length < b + 1)) return a[b];
		};
		x1b = function(a) {
			a.sort((e, f) => e.start - f.start);
			var b = 0;
			var c = 0;
			var d = 0;
			a.forEach((e, f) => {
				if (f === 0) {
					c = e.start, d = e.end;
				} else {
					e.start <= d ? d = Math.max(d, e.end) : (b += d - c, c = e.start, d = e.end);
				}
			});
			return b += d - c;
		};
		nP = function(a) {
			var b = 1;
			for (let d of a) {
				if (d.eventKind === "jsError") a: {
					a = d.jsError;
					if (a.isXhrError === true) {
						a = 1;
						break a;
					}
					if (a.stack) {
						let e = a.stack.substring(a.stack.lastIndexOf("\n"));
						if (a.stack.includes("moz-extension://") || a.stack.includes("chrome-extension://") || e.includes("onmessage")) {
							a = 1;
							break a;
						}
					}
					a = y1b(a.errorExperience);
					a = a !== 0 ? a : 2;
				}
				else a = d.eventKind === "xhr" || d.eventKind === "xhrChunk" ? z1b(d.xhr, d.eventKind === "xhrChunk" ? d.xhrChunk : undefined) : 3;
				if (a > b) {
					b = a;
					var c = d;
					if (b === 3) break;
				}
			}
			return {
				qualityErrorWise: b,
				relevantError: c
			};
		};
		A1b = function(a) {
			return !!a.hasPartialData && (a.unreachableLocations && a.unreachableLocations.length > 0 || !!a.hasServerError);
		};
		z1b = function(a, b) {
			var c = (b || a).errorResponse;
			if (!c) return A1b(a) ? 2 : 1;
			c = y1b(c.errorExperience);
			return c !== 0 ? c : b ? (a = b.statusCode, a === 2 || a === 4 || a === 12 || a === 13 || a === 14 || a === 15 ? 3 : 1) : (a = a.status) ? a >= 500 && a < 600 ? 3 : 1 : 1;
		};
		y1b = function(a) {
			switch (a) {
				case 4:
				case 5: return 3;
				case 6: return 2;
				case 2:
				case 3: return 1;
				default: return 0;
			}
		};
		B1b = function(a) {
			return {
				timestamp: performance.now(),
				eventKind: "jsError",
				jsError: a
			};
		};
		C1b = function(a) {
			return {
				timestamp: performance.now(),
				eventKind: "xhr",
				xhr: a
			};
		};
		D1b = function(a, b) {
			return {
				timestamp: performance.now(),
				eventKind: "xhrChunk",
				xhr: a,
				xhrChunk: b
			};
		};
		G1b = function() {
			if (E1b === undefined) {
				let a = oP.connection || oP.mozConnection || oP.webkitConnection || null;
				if (a && a.addEventListener) {
					let b = new _.Wg();
					a.addEventListener("change", () => {
						var c = F1b();
						if (c) {
							b.next(c);
						}
					});
					E1b = b;
				} else E1b = _.Ef;
			}
			return E1b;
		};
		F1b = function() {
			var a = oP.connection || oP.mozConnection || oP.webkitConnection || null;
			return a ? {
				type: a.type || "unknown",
				effectiveType: a.effectiveType || "unknown",
				downlinkMbps: a.downlink || -1,
				roundTripTimeMs: a.rtt || -1,
				timeStampMs: Date.now()
			} : null;
		};
		H1b = function(a) {
			var b = a.getZoneWith("AsyncTrackingZone");
			return b ? b.parent : a;
		};
		J1b = function(a) {
			var b = { trackForLatency: false };
			var c = I1b().runtimeOptions;
			try {
				I1b().runtimeOptions = Object.assign({}, c, b);
				return a();
			} finally {
				I1b().runtimeOptions = c;
			}
		};
		I1b = function() {
			if (K1b) return K1b;
			for (var a = window;;) try {
				if (a === a.parent) break;
				if (a.parent.__perfmonitor_opts__ !== 123456) {
					a = a.parent;
				}
			} catch (b) {
				break;
			}
			if (!a.__perfmonitor_opts__) {
				a.__perfmonitor_opts__ = { runtimeOptions: {
					track: true,
					trackForLatency: true
				} };
			}
			return K1b = a.__perfmonitor_opts__;
		};
		M1b = function() {
			if (!pP) {
				pP = _.aP() ? sharedHostData.asyncTaskTracker : new L1b();
			}
		};
		P1b = function(a) {
			var b = N1b;
			var c = new _.hk(a).getPath();
			var d = c.replace(/^(\/|)(m|p)\//g, "");
			a = c.replace(/^(\/|)\//g, "").charAt(0);
			if (d !== c) return (b = O1b(d.split("/"), b)) ? `${a}${b}` : "UNRECOGNIZED_PANTHEON_PATH";
		};
		O1b = function(a, b) {
			var c = "";
			var d = 0;
			for (let e of a) {
				if (!b.hasOwnProperty(e)) if (b.hasOwnProperty(".*")) e = ".*";
				else return;
				if (b[e] instanceof Array) {
					let f = b[e];
					if (a.length === d + 1) return `${c}/${e}`;
					a = a.slice(d + 1).map((g) => f.includes(g) ? g : ".*").join("/");
					return `${c}/${e}/${a}`;
				}
				b = b[e];
				c = `${c}/${e}`;
				++d;
			}
			return c;
		};
		R1b = function(a) {
			a = Object.assign({}, a);
			for (let b of Q1b) a.url = b({ url: a.url }).url;
			return a;
		};
		S1b = function(a, b) {
			var c;
			if (b instanceof _.h) {
				c = _.sca(b);
			} else {
				c = b;
			}
			return c;
		};
		T1b = function(a) {
			var b = _.h.prototype.toJSON;
			try {
				_.h.prototype.toJSON = undefined;
				return a();
			} finally {
				_.h.prototype.toJSON = b;
			}
		};
		U1b = function(a) {
			return T1b(() => JSON.stringify(a, S1b, undefined));
		};
		V1b = function(a) {
			return a.split("?")[0].replace(/;[^\/]+/g, "").replace(/\/?#[^\/]+/g, "");
		};
		qP = function(a) {
			return `RellogState_${a[0]}`;
		};
		W1b = function() {
			return _.aP() ? sharedHostData.getZoneCurrentTask() : Zone.currentTask;
		};
		X1b = function(a, b) {
			return {
				sequence: a,
				previousId: b,
				path: "pan-unresolved-page-path",
				id: Math.floor(Math.random() * Number.MAX_SAFE_INTEGER)
			};
		};
		_.rP = function() {
			if (!Y1b) {
				Y1b = new Z1b();
			}
			return Y1b;
		};
		sP = function(a) {
			return `Rellog_${a[0]}`;
		};
		$1b = function(a) {
			var b = RegExp("folder=[^&]", "g");
			var c = RegExp("organizationId=[^&]", "g");
			return RegExp("project=[^&]", "g").test(a) || b.test(a) || c.test(a);
		};
		a2b = function(a) {
			return (a = _.Mv(a, (b) => b.hasAttribute && b.hasAttribute("sandboxid"))) ? (a = a.getAttribute("sandboxid")) ? a : "" : "";
		};
		b2b = function(a) {
			var b = {};
			var c = "";
			if (a) if (a.hasAttribute && a.hasAttribute("sandboxuid")) {
				if (a = a.getAttribute("sandboxuid")) c = (c = _.rP().rifMetadataProvider) ? c.getSandboxidFromSandboxUid(a) || "" : "";
			} else c = a2b(a);
			if (c && _.rP().rifMetadataProvider) return b.p2Metadata = _.rP().rifMetadataProvider.getRifMetadata(c), b.platformType = 5, b;
			c = _.rP();
			if (c.lastPageLoadStopwatch) {
				b.platformType = c.lastPageLoadStopwatch.getPlatformType();
			}
			return b;
		};
		g2b = function(a, b, c, d, e) {
			d = {
				userAction: b,
				target: c.getID(),
				ancestry: d
			};
			var f = { eventType: b ? c2b[b] || "" : "impression" };
			var g = new _.sJ();
			c.mergeTo(g);
			var k = _.Xb(_.tn(g, 4));
			var p = _.Xb(_.tn(g, 3));
			var r = {};
			var v = [];
			var w = _.mj(g, _.rJ, 5, _.oj());
			for (var D of w) {
				if (D.getKey()) {
					r = Object.assign({}, r, { [D.getKey()]: D.getValue() });
				}
				w = D;
				var G = _.Ls(D, _.d2b, 3);
				if (_.wn(w, G) != null) {
					v.push({
						key: _.Lm(D, _.Ls(D, _.d2b, 3)),
						value: D.getValue()
					});
				}
			}
			var L;
			var N;
			D = (N = (L = _.uQa(g, e2b)) == null ? undefined : _.Z(L, tP, 58)) != null ? N : new tP();
			L = new uP();
			c.mergeTo(L);
			if (_.zn(L, 1)) {
				c = _.l(L, 1), _.Lj(D, 1, c);
			}
			if (_.zn(L, 6)) {
				c = _.l(L, 6), _.Lj(D, 2, c);
			}
			d.eventName = k;
			d.eventType = p;
			d.kvMetadata = r;
			d.cloudConsoleVeMetadata = D;
			d.metadataArray = v;
			if (b == null) {
				a.logImpression(d, f, e);
			} else {
				if (f2b.has(b)) {
					a.logOnPageInteraction(d, f, e);
				}
			}
		};
		h2b = function() {};
		i2b = function() {};
		j2b = function(a) {
			this.A = a;
			this.F = null;
		};
		k2b = function() {
			var a = null;
			a = _.ha.sessionStorage || null;
			j2b(a);
		};
		_.Tv.prototype.BJa = _.ca(33, function(a) {
			this.Na = a;
		});
		_.Tv.prototype.CJa = _.ca(32, function(a) {
			this.Ta = a && this.fa;
		});
		_.Tv.prototype.vba = _.ca(31, function(a) {
			this.ta = a;
		});
		_.d2b = [
			3,
			4,
			5,
			6,
			7,
			8,
			9,
			10,
			11,
			12,
			13,
			14,
			15,
			16,
			17,
			18,
			19,
			20
		];
		l2b = class extends _.h {
			constructor(a) {
				super(a, 0, l2b.messageId);
			}
			getFlags() {
				return _.Z(this, _.no, 6, _.gb);
			}
		};
		l2b.messageId = "p.rtres";
		var m2b = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var n2b = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var o2b = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var p2b = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var r2b = function(a) {
			_.mj(a, m2b, 17, _.oj(_.vPa));
		};
		var t2b = function(a) {
			return _.Z(a, o2b, 28);
		};
		var u2b = function(a) {
			return _.Z(a, n2b, 31);
		};
		var uP = class extends _.h {
			constructor(a) {
				super(a, 34);
			}
			DA(a) {
				return _.Us(this, 31, _.rJ, a);
			}
		};
		_.Fs(q1b, _.da);
		q1b.prototype.name = "AssertionError";
		var v2b = _.bd(_.rJ);
		var w2b = {
			YXb: 0,
			e2b: 1,
			f_b: 2,
			YVb: 3,
			v4b: 4,
			AZb: 5,
			TXb: 6,
			I2b: 7,
			c4b: 8,
			XXb: 9,
			g4b: 10,
			L_b: 11,
			BYb: 12
		};
		var y2b = (x2b = Symbol.toStringTag, Symbol.iterator);
		var z2b = function(a, b) {
			var c = a.U6.get(b);
			if (c !== undefined && c <= Date.now()) {
				a.delete(b);
			}
		};
		var A2b = function(a, b) {
			for (let c of [...a.listeners]) try {
				c(b);
			} catch (d) {
				setTimeout(() => {
					throw d;
				});
			}
		};
		var B2b = class {
			constructor(a, b) {
				this.JAb = a;
				this.listeners = [];
				this.U6 = new Map();
				this[x2b] = "ListenableMapImpl";
				this.data = b || new Map();
			}
			get size() {
				this.invalidateAllExpiredEntries();
				return this.data.size;
			}
			addEventListener(a, b) {
				this.listeners.push(b);
			}
			removeEventListener(a, b) {
				a = this.listeners.indexOf(b);
				return a !== -1 ? (this.listeners.splice(a, 1), true) : false;
			}
			clear() {
				var a = [...this.data.entries()];
				this.data.clear();
				this.U6.clear();
				for (let [b, c] of a) A2b(this, {
					key: b,
					value: c,
					type: "REMOVE"
				});
			}
			delete(a) {
				var b = this.data.get(a);
				var c = this.data.delete(a);
				if (c) {
					A2b(this, {
						key: a,
						value: b,
						type: "REMOVE"
					}), this.U6.delete(a);
				}
				return c;
			}
			entries() {
				this.invalidateAllExpiredEntries();
				return this.data.entries();
			}
			forEach(a, b) {
				for (let [c, d] of this.entries()) a.call(b, d, c, this);
			}
			get(a) {
				z2b(this, a);
				return this.data.get(a);
			}
			has(a) {
				z2b(this, a);
				return this.data.has(a);
			}
			keys() {
				this.invalidateAllExpiredEntries();
				return this.data.keys();
			}
			set(a, b, c) {
				if (c = c || this.JAb) {
					this.U6.set(a, Date.now() + c);
				}
				this.data.set(a, b);
				A2b(this, {
					key: a,
					value: b,
					type: "SET"
				});
				return this;
			}
			values() {
				this.invalidateAllExpiredEntries();
				return this.data.values();
			}
			[y2b]() {
				return this.entries();
			}
			invalidateAllExpiredEntries() {
				for (let a of this.U6.keys()) z2b(this, a);
			}
		};
		var C2b;
		var D2b;
		C2b = class {};
		D2b = null;
		_.wP = class {
			static get instance() {
				if (!D2b) {
					D2b = _.aP() ? sharedHostData.globalCacheDataService : new vP("pangular");
				}
				return D2b;
			}
		};
		_.wP.J = function(a) {
			return new (a || _.wP)();
		};
		_.wP.sa = _.Cd({
			token: _.wP,
			factory: () => _.wP.instance,
			wa: "root"
		});
		var vP = class extends _.wP {
			constructor(a) {
				super();
				this.H = a;
				this.F = new Map();
				this.R = new Map();
				this.A = new Map();
				this.I = new _.ml(null);
			}
			getOrCreateListenableMap(a, b = {}, c, d) {
				var e = d != null ? d : this.H;
				if ((d = a !== "NON_SHAREABLE_CACHE_ID") && c) throw Error("Ff");
				if (d) {
					if (!this.A.has(a)) {
						this.A.set(a, new Set());
					}
					let f = this.A.get(a);
					f.add(e);
					if (f.size > 1) {
						this.I.next(new C2b());
					}
				}
				if (this.F.has(a)) return this.F.get(a);
				c = new B2b(b.itemsExpireAfterMs, c);
				if (d) {
					this.F.set(a, c), this.R.set(a, b);
				}
				return c;
			}
		};
		vP.J = function() {
			_.Eg();
		};
		vP.sa = _.Cd({
			token: vP,
			factory: vP.J
		});
		var F2b = (E2b = Symbol.toStringTag, Symbol.iterator);
		var xP = function(a) {
			var b;
			var c;
			if (!((c = (b = a.rr).invalidateAllExpiredEntries) == null)) {
				c.call(b);
			}
			for (let d of a.rr.keys()) b = a, b.jO.TBa && b.jO.TBa(d) && b.delete(d);
		};
		var G2b = function(a) {
			return _.zf(() => a.sT("globalSubjectStateKey", false).pipe(_.bh(a.get("globalSubjectStateKey")), _.Gf((b) => true)));
		};
		var H2b = class {
			constructor(a, b = {}) {
				this.rr = a;
				this.jO = b;
				this[E2b] = "StateV2";
				this.zf = _.Af(this.rr, "change").pipe(_.uf((c) => ({
					key: c.key,
					value: this.yI(c.value),
					action: c.type
				})));
			}
			get(a) {
				if (this.has(a)) return a = this.rr.get(a), a === undefined ? undefined : this.yI(a);
			}
			has(a) {
				if (this.jO.TBa && this.jO.TBa(a)) {
					this.delete(a);
				}
				return this.rr.has(a);
			}
			set(a, b, c) {
				b = this.jO.Wn ? this.jO.Wn.serialize(b) : b;
				this.rr.set(a, b, c);
				return this;
			}
			delete(a) {
				return this.rr.delete(a);
			}
			clear() {
				this.rr.clear();
			}
			sT(a, b = false) {
				var c = this.zf.pipe(_.Gf((d) => a ? d.key === a : true), _.uf((d) => d.action === "REMOVE" ? undefined : d.value));
				return b ? c.pipe(_.Gf((d) => d !== undefined)) : c;
			}
			get size() {
				xP(this);
				return this.rr.size;
			}
			forEach(a, b) {
				xP(this);
				for (let [c, d] of this.entries()) a.call(b, d, c, this);
			}
			keys() {
				xP(this);
				return this.rr.keys();
			}
			values() {
				var a = this;
				return function* () {
					xP(a);
					for (let b of a.rr.values()) yield a.yI(b);
				}();
			}
			entries() {
				var a = this;
				return function* () {
					xP(a);
					for (let [b, c] of a.rr.entries()) yield [b, a.yI(c)];
				}();
			}
			yI(a) {
				return a === undefined || a === null ? a : this.jO.Wn ? this.jO.Wn.deserialize(a) : a;
			}
			[F2b]() {
				xP(this);
				return this.entries();
			}
		};
		_.yP = class {
			constructor(a, b) {
				this.initialValue = b;
				a = _.wP.instance.getOrCreateListenableMap(a);
				this.rr = new H2b(a);
				if (b && !this.rr.has("globalValueStateKey")) {
					this.set(b);
				}
			}
			get() {
				var a = this.rr.get("globalValueStateKey");
				return a !== undefined ? a : undefined;
			}
			set(a) {
				if (_.Hf(a)) throw Error("Gf");
				this.rr.set("globalValueStateKey", a);
				return this;
			}
			delete() {
				return this.rr.delete("globalValueStateKey");
			}
		};
		var I2b = function(a) {
			return a.config.variant === "BEHAVIOR_SUBJECT";
		};
		var J2b = class {
			constructor(a, b = { variant: "SUBJECT" }) {
				this.A = a;
				this.config = b;
				if (I2b(this)) {
					this.A.has("globalSubjectStateKey") || this.A.set("globalSubjectStateKey", this.config.initialValue);
				}
			}
			next(a) {
				this.A.set("globalSubjectStateKey", a);
			}
			subscribe(a) {
				return this.asObservable().subscribe(a);
			}
			asObservable() {
				return (this.config.variant === "SINGLE_REPLAY_SUBJECT" || I2b(this) ? G2b(this.A) : this.A.sT("globalSubjectStateKey")).pipe(_.Gf(() => this.A.has("globalSubjectStateKey")));
			}
			get PDa() {
				if (this.config.variant !== "SINGLE_REPLAY_SUBJECT" && !I2b(this)) throw new q1b("GlobalSubject value can only be fetched when using the SINGLE_REPLAY or BEHAVIOR variants.", []);
				if (!this.A.has("globalSubjectStateKey")) throw new q1b("Subject has not been initialized with value", []);
				return this.A.get("globalSubjectStateKey");
			}
			error() {}
			complete() {}
		};
		var K2b;
		K2b = null;
		_.zP = function(a, b, c = { variant: "SUBJECT" }) {
			a = a.globalCacheDataService.getOrCreateListenableMap(b, {}, undefined);
			a = new H2b(a, {});
			return new J2b(a, c);
		};
		_.AP = class {
			static get instance() {
				if (!K2b) {
					K2b = new _.AP(_.wP.instance);
				}
				return K2b;
			}
			constructor(a) {
				this.globalCacheDataService = a;
			}
		};
		_.AP.J = function(a) {
			return new (a || _.AP)(_.ae(_.wP));
		};
		_.AP.sa = _.Cd({
			token: _.AP,
			factory: () => _.AP.instance,
			wa: "root"
		});
		var L2b;
		var M2b = class extends _.Wg {
			constructor() {
				super();
				this.U = 2;
				this.F = [];
				this.R = _.yf(_.Ef.pipe(_.eh({ complete: () => {
					this.X();
				} })), this.asObservable());
			}
			X() {
				if (!--this.U) {
					setTimeout(() => {
						for (let a of this.F) super.next(a);
						this.F = [];
					}, 0);
				}
			}
			next(a) {
				if (this.U > 0) {
					this.F.push(a);
				} else {
					super.next(a);
				}
			}
		};
		var N2b = function(a) {
			if (a.R) {
				a.R.unsubscribe();
			}
			return a.pageHidden;
		};
		var BP = function(a) {
			return a.startTimeMs + Math.floor(performance.now() - a.performanceStartTimeMs);
		};
		var CP = function(a, b) {
			return a.startTimeMs + Math.floor(b - a.performanceStartTimeMs);
		};
		var DP = class {
			constructor() {
				this.startTimeMs = Date.now();
				this.performanceStartTimeMs = performance.now();
				this.pageHidden = !!document.hidden;
				if (!this.pageHidden) {
					if (!L2b) {
						L2b = v1b();
					}
					let a = L2b.pipe(_.Ug()).subscribe(() => {
						this.pageHidden = true;
						a.unsubscribe();
					});
					this.R = a;
				}
			}
			setPlatformType(a) {
				if (!(this.platformType && this.platformType !== 1 && this.platformType !== 2)) {
					this.platformType = a;
				}
			}
			getPlatformType() {
				return this.platformType;
			}
			setP2Metadata(a) {
				this.p2Metadata = a;
			}
			getP2Metadata() {
				return this.p2Metadata;
			}
			tick() {
				return BP(this) - this.startTimeMs;
			}
			tickInterval() {
				var a = BP(this);
				return {
					start: this.startTimeMs,
					end: a
				};
			}
			elapsed() {
				return this.endTimeMs - this.startTimeMs;
			}
			elapsedInterval() {
				return {
					start: this.startTimeMs,
					end: this.endTimeMs
				};
			}
			stop() {
				this.endTimeMs = BP(this);
				return this;
			}
			isStopped() {
				return this.endTimeMs !== undefined;
			}
		};
		var O2b = class extends DP {
			stopAndRecord() {
				this.stop();
			}
			started() {
				return this.startTimeMs;
			}
			stopped() {
				return this.endTimeMs;
			}
		};
		var P2b = class extends DP {
			constructor() {
				super(...arguments);
				this.F = [];
				this.xhrTimings = [];
				this.encryptedTaskInfo = new Set();
			}
			addInteractionError(a) {
				this.F.push(a);
			}
			addXhrTiming(a, b) {
				this.xhrTimings.push({
					start: a,
					end: b
				});
			}
			addEncryptedTaskInfo(a) {
				for (let b of a) this.encryptedTaskInfo.add(b);
			}
		};
		var Q2b = class {
			constructor(a, b, c) {
				this.origin = a;
				this.emit = b;
				this.F = false;
				this.metrics = [];
				this.Hs = setTimeout(() => {
					this.send();
				}, c);
			}
			get A() {
				return this.F;
			}
			send() {
				if (!this.F) {
					clearTimeout(this.Hs), this.F = true, this.emit(this.metrics), this.metrics = [];
				}
			}
			record(a) {
				if (!this.A) {
					this.metrics.push(a);
				}
			}
		};
		var R2b = class {
			constructor(a) {
				this.state = a;
			}
			get I() {
				var a = EP(this.state).origin.pageView;
				return this.A && !this.A.A && this.A.origin.pageView.sequence === a.sequence;
			}
			record(a, b) {
				if (!this.I) {
					this.A && !this.A.A && (this.A.send(), this.A = undefined), this.A = b();
				}
				this.A.record(a);
			}
		};
		var S2b = {
			[0]: "unknown_metric",
			[1]: "js_module_load",
			[2]: "js_module_fetch",
			[3]: "js_module_load_sum",
			[4]: "js_module_fetch_sum",
			[5]: "xhr_sum",
			[6]: "rest_sum",
			[7]: "page_load_primary",
			[8]: "route_transition_lazy_loaded",
			[303]: "route_transition_guards_checked",
			[9]: "route_transition_data_resolved",
			[10]: "route_transition_initial",
			[11]: "route_transition",
			[12]: "account_chooser_loaded",
			[13]: "activity_fetch",
			[14]: "activity_first_render",
			[15]: "activity_first_request",
			[16]: "activity_first_response",
			[17]: "activity_infini_fetch",
			[18]: "activity_infini_render",
			[19]: "activity_render",
			[310]: "agreements_guard",
			[319]: "ai_chat_create_topic",
			[322]: "ai_chat_create_topic_operation",
			[321]: "ai_chat_send_message_full",
			[320]: "ai_chat_send_message",
			[324]: "ai_chat_start_chat_panel",
			[20]: "angular_bootstrapped",
			[21]: "app_bootstrapped",
			[22]: "app_loaded",
			[23]: "athos_list_recommendations",
			[24]: "bigquery_prefetch",
			[305]: "billing_enabled_guard",
			[25]: "card_api_graphs",
			[26]: "card_appengine_graphs",
			[27]: "card_billing",
			[28]: "card_cloud_news",
			[29]: "card_cloud_status",
			[30]: "card_compute_graphs",
			[31]: "card_crash",
			[32]: "card_getting_started",
			[33]: "card_monitoring",
			[34]: "card_monitoring_deferred",
			[35]: "card_project_info",
			[36]: "card_project_info_deferred",
			[37]: "card_resources",
			[38]: "card_resources_deferred",
			[39]: "card_sql_graphs",
			[40]: "card_support_docs",
			[41]: "card_trace",
			[42]: "chrome_header_visible",
			[43]: "chrome_leftnav_ready",
			[44]: "chrome_leftnav_visible",
			[45]: "clouddev_aliases_read",
			[46]: "clouddev_autosave_change",
			[47]: "clouddev_autosave_last_change",
			[48]: "clouddev_autosave_write",
			[49]: "clouddev_create_commit_rpc",
			[50]: "clouddev_refresh_workspace_rpc",
			[51]: "clouddev_revert_file",
			[52]: "clouddev_revert_workspace",
			[53]: "clouddev_source_read",
			[54]: "core_executed",
			[55]: "dashboard_all_cards",
			[56]: "dashboard_all_cards_perf",
			[57]: "dashboard_initial",
			[58]: "dashboard_most_cards",
			[59]: "dashboard_most_cards_perf",
			[60]: "dashboard_preferences",
			[61]: "dashboard_some_cards",
			[62]: "dashboard_some_cards_perf",
			[63]: "dashboard_visible",
			[64]: "datatable_fetch",
			[65]: "digest_after_route_change",
			[311]: "dynamic_rif_guard",
			[66]: "dom_content_loaded",
			[318]: "easy_apis_signup",
			[67]: "free_trial_billing_submission",
			[325]: "free_trial_tos_to_billing_form",
			[326]: "free_trial_signup_get_next_step_decision",
			[327]: "free_trial_skip_tos_redirect_guard",
			[328]: "free_trial_prepare_for_billing_redirect_guard",
			[68]: "gae_app_basic_get",
			[69]: "gae_app_get",
			[70]: "gae_authorized_certificates_get",
			[71]: "gae_authorized_certificates_list",
			[72]: "gae_authorized_domains_list",
			[73]: "gae_billing_state",
			[74]: "gae_certificates_get",
			[75]: "gae_certificates_list",
			[76]: "gae_code_lock",
			[77]: "gae_crons_list",
			[78]: "gae_custom_domains_list",
			[79]: "gae_dashboard_billing",
			[80]: "gae_dashboard_crash",
			[81]: "gae_dashboard_stats",
			[82]: "gae_domains_app_list",
			[83]: "gae_domains_owner_list",
			[84]: "gae_flex_instances_list",
			[85]: "gae_memcache",
			[86]: "gae_queues_get",
			[87]: "gae_queues_list",
			[88]: "gae_quota",
			[89]: "gae_quota_limit",
			[90]: "gae_services_detail_list",
			[91]: "gae_services_list",
			[92]: "gae_standard_instances_list",
			[93]: "gae_tasks_list",
			[94]: "gae_usage_history",
			[95]: "gae_versions_all_data_load",
			[96]: "gae_versions_list",
			[97]: "gae_versions_table_load",
			[288]: "gcf_predeployment_testing",
			[98]: "gallery_loaded",
			[99]: "gce_api_enable",
			[100]: "gce_load_resources",
			[101]: "gcr_images_additional_columns",
			[102]: "gcr_images_summary_tab",
			[103]: "gcr_images_vulnz_tab",
			[323]: "get_hidden_feature_areas",
			[104]: "get_project",
			[309]: "iam_permissions_guard",
			[307]: "iam_permissions_extended_guard",
			[105]: "initial_data_first_packet",
			[106]: "initial_data_last_packet",
			[107]: "initial_settings_packet",
			[108]: "js_active",
			[280]: "js_code_decoded_size",
			[281]: "js_code_encoded_size",
			[282]: "js_code_transfer_size",
			[109]: "labels_hashes_calculate",
			[110]: "landing_page_navigation",
			[111]: "late_data",
			[112]: "lazy_module_load_finished",
			[113]: "logs_fetch_to_first_result",
			[114]: "logs_fetch_to_search_done",
			[115]: "logs_log_name_fetch",
			[116]: "logs_mc_init_to_first_fetch",
			[117]: "logs_mc_init_to_l_names_ready",
			[118]: "logs_mc_init_to_resources_ready",
			[119]: "logs_resource_selector_setup",
			[120]: "logs_rt_start_to_main_ctrl_init",
			[121]: "longest_task_duration",
			[122]: "long_post_ng_page_loaded_digest_time",
			[274]: "long_task",
			[123]: "long_task_post_ng_page_loaded",
			[124]: "lpx_billing_full",
			[125]: "lpx_billing_interactive",
			[126]: "lpx_billing_rendered",
			[127]: "lpx_billing_streamed",
			[128]: "lpx_incidents_full",
			[129]: "lpx_incidents_rendered",
			[130]: "lpx_incidents_streamed",
			[131]: "lpx_quotas_full",
			[132]: "lpx_quotas_rendered",
			[133]: "lpx_quotas_streamed",
			[134]: "lpx_status_full",
			[135]: "lpx_status_rendered",
			[136]: "lpx_status_streamed",
			[137]: "marketplace_instance_create",
			[138]: "marketplace_price_est",
			[139]: "marketplace_project_enable",
			[140]: "marketplace_project_select",
			[141]: "marketplace_solution_launch",
			[142]: "marketplace_sum",
			[143]: "metropolis_dashboard_table_load",
			[144]: "module_manager_initialized",
			[145]: "module_preload",
			[146]: "module_preload_headstart",
			[147]: "monitoring_appstats_query",
			[148]: "monitoring_monarch_query",
			[149]: "mp_config_apis_enabled",
			[150]: "mp_config_dm_enabled",
			[151]: "mp_config_gce_enabled",
			[152]: "mp_config_organizations_loaded",
			[153]: "mp_config_resources_loaded",
			[154]: "mp_config_runtime_enabled",
			[155]: "mp_config_solution_loaded",
			[156]: "mp_vm_deploy_config_loaded",
			[157]: "mp_vm_deploy_config_ready",
			[158]: "mp_vm_product_config_loaded",
			[159]: "mp_vm_product_config_ready",
			[160]: "mycloud_legacy_apis_load_data",
			[161]: "mycloud_update_card_grid",
			[162]: "mycloud_update_resource_tree",
			[315]: "perf_nav_server_timing_gfe_l1",
			[316]: "perf_nav_server_timing_gfe_l2",
			[283]: "perf_nav_timing_fetch",
			[284]: "perf_nav_timing_request",
			[285]: "perf_nav_timing_response",
			[163]: "navigation_full_page_error",
			[164]: "ncon_search_results",
			[165]: "ng_page_loaded",
			[166]: "operation_complete",
			[167]: "p2_feature_content_guards_block",
			[168]: "p2_feature_content_guards_run",
			[169]: "p2_feature_content_resolve",
			[170]: "p2_feature_content_resolvers_run",
			[171]: "p2_feature_loader_configure_nav",
			[172]: "p2_feature_loader_load_routes",
			[174]: "page",
			[175]: "page_transition",
			[176]: "page_transition_abandoned",
			[290]: "pan_full_latency",
			[300]: "pan_latency_user_interaction",
			[317]: "pan_latency_tracking_mode",
			[291]: "pan_primary_latency",
			[177]: "pangolin_boot_api_rendering_done",
			[178]: "pangolin_boot_preload_to_active",
			[179]: "pangolin_boot_proc_total",
			[180]: "pangolin_boot_rcv_first_message",
			[181]: "pangolin_boot_root_layout_init",
			[182]: "pangolin_boot_root_rendered",
			[183]: "pangolin_boot_sandbox_bootstrap",
			[184]: "pangolin_boot_sandbox_comp_init",
			[185]: "pangolin_boot_sandbox_preloaded",
			[186]: "pangolin_boot_total",
			[187]: "pangolin_boot_total_no_gaps",
			[188]: "pangolin_boot_worker_code_ready",
			[189]: "pangolin_boot_worker_init_start",
			[190]: "permissions_on_resolve",
			[191]: "pinned_section_loaded",
			[192]: "platformbar_settings_loaded",
			[193]: "post_success_digest_time",
			[194]: "post_success_long_tasks",
			[195]: "profiler_history_view_load",
			[196]: "profiler_init_to_page_load",
			[197]: "profiler_init_to_profile_load",
			[198]: "profiler_profile_query_to_load",
			[306]: "project_permissions_guard",
			[199]: "project_selector_complete",
			[304]: "purview_change_redirect_guard",
			[200]: "purview_permissions_guard",
			[201]: "purview_picker_all_tab_list_view",
			[202]: "purview_picker_all_tab_tree_view",
			[203]: "purview_picker_recent_tab_list",
			[204]: "require_purview_guard",
			[313]: "resolver_coliseum_table_manager",
			[301]: "resolver_iam_member_list",
			[314]: "resolver_iam_permissions",
			[302]: "resolver_iam_roles",
			[312]: "resolver_project_permissions",
			[205]: "resource_loading_time",
			[293]: "resource_size_accounts",
			[294]: "resource_size_firewall_rules",
			[295]: "resource_size_instances",
			[299]: "resource_size_projects",
			[296]: "resource_size_services",
			[297]: "resource_size_triggers",
			[298]: "resource_size_versions",
			[206]: "rif_initialize",
			[207]: "rif_load_base_module",
			[208]: "rif_load_iframe",
			[273]: "rif_started",
			[209]: "route_change_finished",
			[210]: "route_change_start",
			[287]: "route_redirect_render",
			[211]: "route_transition_abandoned",
			[212]: "route_transition_interactive",
			[213]: "route_transition_modules_fetched",
			[214]: "search_cluster_load",
			[215]: "search_final_cluster_load",
			[216]: "search_initial_cluster_load",
			[217]: "search_open_to_selection",
			[218]: "search_query_to_selection",
			[219]: "search_results_displayed",
			[220]: "server_response",
			[308]: "service_enabled_guard",
			[221]: "shell_account_chooser_load",
			[222]: "shell_account_chooser_open",
			[223]: "shell_console_nav_button_load",
			[224]: "shell_console_nav_data_init",
			[225]: "shell_console_nav_deferred_load",
			[226]: "shell_console_nav_first_open",
			[227]: "shell_console_nav_other_open",
			[228]: "shell_dev_shell_button_load",
			[229]: "shell_dev_shell_service_load",
			[230]: "shell_feedback_button_load",
			[231]: "shell_feedback_service_load",
			[232]: "shell_free_trial_button_load",
			[233]: "shell_help_button_load",
			[234]: "shell_help_service_load",
			[235]: "shell_notifications_button_load",
			[236]: "shell_notifications_menu_load",
			[237]: "shell_pinned_sections_load",
			[238]: "shell_platform_bar_load",
			[239]: "shell_purview_button_load",
			[240]: "shell_routes_recognized_cancel",
			[241]: "shell_routes_recognized_failed",
			[242]: "shell_routes_recognized_success",
			[243]: "shell_search_bar_load",
			[244]: "shell_search_perceived_load",
			[245]: "shell_search_results_load",
			[246]: "shell_section_config_load",
			[247]: "shell_section_nav_defer_loaded",
			[248]: "shell_section_pages_initialized",
			[249]: "shell_settings_button_load",
			[275]: "shell_workspace_button_load",
			[250]: "sql_insights_get_query_plan",
			[251]: "sql_insights_get_query_samples",
			[276]: "sql_insights_get_recommendations",
			[277]: "sql_insights_get_recommendation_stats",
			[252]: "sql_insights_get_top_clients",
			[253]: "sql_insights_get_top_queries",
			[254]: "sql_insights_get_top_tags",
			[255]: "sql_insights_get_top_users",
			[256]: "support_get_package",
			[257]: "support_list_user_roles",
			[258]: "support_set_user_roles",
			[259]: "support_zero_state_resolve",
			[260]: "targeted_emergency_messaging",
			[261]: "topology_diff_draw",
			[262]: "topology_first_draw",
			[263]: "topology_redraw",
			[264]: "user_preferences_configs_load",
			[265]: "view_content_loaded",
			[266]: "welcome_default_path_guard",
			[267]: "welcome_landing_page_brief_scope",
			[268]: "welcome_landing_page_cards",
			[289]: "welcome_landing_page_header",
			[269]: "welcome_landing_page_home_links",
			[270]: "welcome_landing_page_purview_resolve",
			[271]: "welcome_landing_page_quick_access_resolve",
			[272]: "welcome_landing_page_quick_actions",
			[279]: "welcome_new_user_landing_redirect",
			[292]: "new_user_landing_free_trial_status",
			[278]: "largest_contentful_paint",
			[286]: "cumulative_layout_shift"
		};
		var FP = {};
		for (let a of Object.keys(S2b)) FP[S2b[a]] = Number(a);
		var V2b = class {
			constructor() {
				this.A = Math.floor(Math.random() * 1e3).toString(26);
			}
			mark(a = "", b, c = false, d) {
				var e = performance;
				if (!e || !e.mark) return "";
				if (!(b || T2b < U2b + 100)) {
					U2b = T2b;
					var f = performance;
					if (f && f.clearMarks && f.clearMeasures && f.getEntries && f.getEntries().length > 5e3) {
						f.clearMarks(), f.clearMeasures();
					}
				}
				c = c ? a : `${a}:${this.A}:${T2b++}`;
				if (d === undefined || isNaN(d) || !isFinite(d) || d < 0) d = e.now();
				e.mark(c, { startTime: d });
				if (b !== undefined) {
					this.measure(a, b, c);
				}
				return c;
			}
			measure(a, b, c) {
				var d = performance;
				if (d && d.measure) d.measure(a, b, c || a);
			}
		};
		var T2b = 0;
		var U2b = 0;
		var W2b = class extends Q2b {
			constructor(a, b) {
				super(a, (c) => {
					b({
						customMetrics: c,
						pageView: a.pageView,
						interaction: a.interaction
					});
				}, 1e4);
			}
		};
		var Y2b = class extends R2b {
			constructor(a, b) {
				super(b);
				this.F = a;
			}
			startCustomEvent(a, b = 0) {
				return new X2b(b, a, EP(this.state).origin, (c, d) => {
					this.record(c, () => new W2b(d, (e) => {
						this.F.next(e);
					}));
				});
			}
			logCustomEvent(a, b, c, d) {
				if (a in FP) {
					this.record({
						eventKey: FP[a],
						startTimeMs: Math.floor(b.start),
						endTimeMs: Math.floor(b.end),
						longTask: c,
						resourceSize: d
					}, () => new W2b(EP(this.state).origin, (e) => {
						this.F.next(e);
					}));
				}
			}
		};
		var X2b = class extends DP {
			constructor(a, b, c, d) {
				super();
				this.label = b;
				this.origin = c;
				this.F = d;
				this.A = GP || (GP = new V2b());
				b = this.origin.pageView;
				if (a !== 0 && b.timings) {
					a === 1 ? this.startTimeMs = b.timings.startTimeMs : a === 2 && (this.startTimeMs = b.timings.lastRedirectTimeMs), this.performanceStartTimeMs = performance.now() - (Date.now() - this.startTimeMs);
				}
			}
			record(a, b) {
				a = a != null ? a : this.label;
				if (a in FP) {
					this.F({
						eventKey: FP[a],
						startTimeMs: this.startTimeMs,
						endTimeMs: this.isStopped() ? this.endTimeMs : BP(this),
						longTask: undefined,
						resourceSize: b
					}, this.origin), this.hasOwnProperty("userTiming") && this.A !== undefined && (b = this.A.mark("", undefined, false, this.performanceStartTimeMs), this.A.mark(a, b));
				}
			}
			stopAndRecord(a, b) {
				this.stop();
				this.record(a, b);
			}
		};
		var Z2b = Symbol("Hf");
		var $2b = class {
			constructor(a, b) {
				this.A = a;
				this.state = b;
			}
			logJsError(a, b = this.state.getPlatformMetadata()) {
				a = Object.assign({}, a, {
					message: a.message && a.message.length > 1024 ? a.message.substr(0, 1024) : a.message,
					stack: a.stack && a.stack.length > 51200 ? a.stack.substr(0, 51200) : a.stack
				});
				if (!(a.message === "Script error" || a.message === "Script error." || a.message === "Error: Script error" || a.message === "Error: Script error." || a.message && a.message.includes("ErrorResponse: {\"errorParameters\":"))) {
					var { origin: c, interactionStopwatch: d } = EP(this.state);
					var e = true;
					if (d) {
						d.addInteractionError(B1b(a)), e = false;
					}
					e = e ? undefined : c.interaction;
					let f = nP([B1b(a)]);
					b = Object.assign({}, a, {
						pageView: c.pageView,
						interaction: e,
						qualityErrorWise: f.qualityErrorWise,
						originAccurate: c.accurate,
						platformMetadata: b
					});
					this.A.next(b);
				}
			}
		};
		var a3b = class extends DP {
			constructor(a, b, c, d, e) {
				super();
				this.ids = a;
				this.origin = b;
				this.U = c;
				this.F = d;
				this.I = e;
				this.A = GP || (GP = new V2b());
				this.prefetchedModules = [];
				this.prefetchedModuleHits = [];
				this.H = this.decodedModuleSizeInBytes = undefined;
				if (this.I) {
					let f = new RegExp(`\\b(${a.join("|")})\\b`, "g");
					this.H = this.I.pipe(_.Ula(() => !this.isStopped()), _.Gf((g) => f.test(g.name))).subscribe((g) => {
						this.decodedModuleSizeInBytes = (this.decodedModuleSizeInBytes || 0) + g.decodedBodySize;
					});
				}
			}
			stopAndRecord({ sendToUserTiming: a = true } = {}) {
				var b;
				if (!((b = this.H) == null)) {
					b.unsubscribe();
				}
				this.H = undefined;
				b = BP(this);
				b = {
					ids: this.ids,
					latencyMs: b - this.startTimeMs,
					startTimeMs: this.startTimeMs,
					performanceStartTimeMs: this.performanceStartTimeMs,
					endTimeMs: b,
					prefetchedModules: this.prefetchedModules,
					prefetchedModuleHits: this.prefetchedModuleHits,
					decodedModuleSizeInBytes: this.decodedModuleSizeInBytes
				};
				if (this.F && !this.F.isStopped()) {
					this.F.recordJsModuleLoad(b);
				} else {
					this.U(b, this.origin);
				}
				if (a) {
					a = this.A.mark("", undefined, false, b.performanceStartTimeMs), this.A.mark("lazy_module_load_finished", a), this.A.mark(`module: ${b.ids.join(",")}`, a);
				}
			}
			logJsModulePrefetch(a, b) {
				this.prefetchedModules.push({
					moduleId: a,
					prefetchSource: b
				});
			}
			logJsModulePrefetchHit(a, b) {
				this.prefetchedModuleHits.push({
					moduleId: a,
					prefetchSource: b
				});
			}
		};
		var b3b = class extends Q2b {
			constructor(a, b, c) {
				super(a, (d) => {
					b({
						jsModuleLoadMetrics: d,
						pageView: a.pageView,
						originAccurate: a.accurate,
						platformMetadata: c
					});
				}, 15e3);
			}
		};
		var c3b = class extends R2b {
			constructor(a, b) {
				super(b);
				this.H = a;
				this.F = new _.Wg();
			}
			startJsModuleLoad(a) {
				return new a3b(a, EP(this.state).origin, (b, c) => {
					this.record(b, () => new b3b(c, (d) => {
						this.H.next(d);
					}, this.state.getPlatformMetadata()));
				}, this.state.lastPageLoadStopwatch, this.F);
			}
		};
		var d3b = RegExp("_/js/|_/mss/|(\\.js$)", "i");
		var E1b;
		var f3b;
		var HP;
		var e3b;
		var g3b;
		var h3b;
		f3b = function(a, b) {
			a.w4--;
			e3b(a);
			a.y4a(b);
		};
		HP = function(a) {
			a.Az.run(() => {});
		};
		e3b = function(a) {
			a.dirty = true;
			if (!a.zbb) {
				a.hca == null && a.w4 === 0 && a.active && a.Az.run(() => {
					a.hca = setTimeout(() => {
						a.active = false;
						a.cma("Stable.");
						a.gSb();
					});
				});
			}
		};
		g3b = function(a) {
			if (!a.zbb) {
				a.hca !== null && a.Az.run(() => {
					clearTimeout(a.hca);
					a.hca = null;
				});
			}
		};
		h3b = function(a, b) {
			return b.type !== "macroTask" || b.data && b.data.isPeriodic ? false : b.data && b.data.delay && b.data.delay >= 1e3 ? (HP(a), false) : true;
		};
		_.i3b = class {
			constructor(a, b, c) {
				this.id = a;
				this.Az = b;
				this.gSb = c;
				this.hca = this.logger = null;
				this.active = true;
				this.zbb = 0;
				this.dirty = false;
				this.w4 = 0;
				this.name = "AsyncTrackingZone";
				this.properties = {};
				this.YKb = null;
				this.cma(`Creating new zone spec [id = ${this.id}]`);
				this.properties.AsyncTrackingZone = this;
			}
			get pendingTasks() {
				return this.w4;
			}
			track(a, b = "") {
				var c = () => {
					f3b(this, `track<${b}>`);
					HP(this);
				};
				HP(this);
				this.w4++;
				g3b(this);
				a.then(c, c);
				return a;
			}
			y4a() {}
			onInvoke(a, b, c, d, e, f, g) {
				try {
					return a.invoke(c, d, e, f, g);
				} finally {
					e3b(this);
				}
			}
			onScheduleTask(a, b, c, d) {
				if (d.type === "eventTask" || d.data && d.data.isPeriodic && d.data.delay === 1333) return d.cancelScheduleRequest(), H1b(c).scheduleTask(d);
				if (h3b(this, d)) {
					HP(this), this.w4++, g3b(this);
				} else {
					HP(this);
				}
				return a.scheduleTask(c, d);
			}
			onCancelTask(a, b, c, d) {
				try {
					return a.cancelTask(c, d);
				} finally {
					if (h3b(this, d)) {
						HP(this), f3b(this, d);
					} else {
						HP(this);
					}
				}
			}
			onInvokeTask(a, b, c, d, e, f) {
				b = h3b(this, d);
				HP(this);
				try {
					return a.invokeTask(c, d, e, f);
				} finally {
					if (b) {
						f3b(this, d);
					}
				}
			}
			cma() {
				this.Az.run(() => {});
			}
			setRootZone(a) {
				this.Az = a;
			}
		};
		var j3b = class {
			constructor(a, b, c = false) {
				this.H = a;
				this.Az = b;
				this.F = c;
				this.A = false;
			}
			now() {
				return Date.now();
			}
			Me(a, b = 0, c) {
				return this.A ? _.KSa.Me(a, b, c) : (this.F || this.Az.run(() => {
					setTimeout(() => {
						this.A = true;
					}, 0);
				}), this.H.run(() => _.KSa.Me(a, b, c)));
			}
		};
		var L1b = class {
			constructor() {
				this.R = new _.Wg();
				this.F = null;
				this.I = this.Az = Zone.root;
				this.logger = null;
				this.onStable = this.R.asObservable();
				if (_.aP()) throw Error("If");
				this.reset(-1);
			}
			reset(a) {
				this.A = new _.i3b(a, this.Az, () => {
					this.R.next(a);
				});
				this.H = this.Az.fork(this.A);
				this.F = null;
				this.A.YKb = this.H;
				return this.A;
			}
			get activeTrackingZone() {
				var a = Zone.current.getZoneWith("isAngularZone");
				return a ? (this.F || (this.F = a.fork(this.A)), this.F) : this.H;
			}
			getActiveTrackingZoneSpec() {
				return this.getZoneSpec(this.activeTrackingZone);
			}
			isInTrackingZone() {
				return Zone.current.get("AsyncTrackingZone") != null;
			}
			isInActiveTrackingZone() {
				return this.maybeGetZoneSpec() === this.getActiveTrackingZoneSpec();
			}
			run(a) {
				var b = this.activeTrackingZone;
				return b === Zone.current ? a() : b.run(a);
			}
			runOutsideTracker(a) {
				var b = H1b(Zone.current);
				return b && b !== Zone.current ? J1b(() => b.run(a)) : J1b(a);
			}
			track(a, b = "") {
				this.A.track(a, b);
			}
			trackOnCurrentZone(a, b) {
				var c = this.maybeGetZoneSpec();
				return c ? c.track(a, b) : a;
			}
			trackObservableOnCurrentZone(a = false, b = false, c) {
				var d = new _.gw();
				var e = this.I.run(() => {
					var g = setTimeout(() => {
						if (g) {
							c && c(), d.reject("timeout");
						}
					}, 1e4);
					return () => {
						clearTimeout(g);
						g = undefined;
					};
				});
				if (b) {
					this.track(d.promise);
				} else {
					this.trackOnCurrentZone(d.promise);
				}
				var f = new j3b(b ? this.activeTrackingZone : Zone.current, this.I, a);
				return (g) => g.pipe(_.eh(() => {
					if (!a) {
						d.resolve(), e();
					}
				}, () => {
					e();
					d.reject();
				}), _.Tg(() => {
					e();
					d.resolve();
				}), _.Dha(f));
			}
			getZoneSpec(a = Zone.current) {
				return this.maybeGetZoneSpec(a);
			}
			maybeGetZoneSpec(a = Zone.current) {
				return a.get("AsyncTrackingZone");
			}
			setRootZone(a, b = false) {
				this.Az = a;
				if (b) {
					this.I = a;
				}
				this.A.setRootZone(a);
				this.H = this.Az.fork(this.A);
				this.F = null;
			}
			get pendingTasks() {
				return this.A.pendingTasks;
			}
		};
		var k3b;
		var pP;
		var IP = class {
			get onStable() {
				return pP.onStable;
			}
			constructor() {
				M1b();
				return pP;
			}
			reset(a) {
				return pP.reset(a);
			}
			get activeTrackingZone() {
				return pP.activeTrackingZone;
			}
			getActiveTrackingZoneSpec() {
				return pP.getActiveTrackingZoneSpec();
			}
			isInTrackingZone() {
				return pP.isInTrackingZone();
			}
			isInActiveTrackingZone() {
				return pP.isInActiveTrackingZone();
			}
			run(a) {
				return pP.run(a);
			}
			runOutsideTracker(a) {
				return pP.runOutsideTracker(a);
			}
			track(a, b) {
				pP.track(a, b);
			}
			trackObservableOnCurrentZone(a = false, b = false, c) {
				return pP.trackObservableOnCurrentZone(a, b, c);
			}
			trackOnCurrentZone(a, b) {
				return pP.trackOnCurrentZone(a, b);
			}
			getZoneSpec(a) {
				return pP.getZoneSpec(a);
			}
			maybeGetZoneSpec(a) {
				return pP.maybeGetZoneSpec(a);
			}
			setRootZone(a, b) {
				pP.setRootZone(a, b);
			}
			get pendingTasks() {
				return pP.pendingTasks;
			}
		};
		IP.J = function(a) {
			return new (a || IP)();
		};
		IP.sa = _.Cd({
			token: IP,
			factory: () => {
				M1b();
				return pP;
			},
			wa: "root"
		});
		var N1b = {
			abusiveprojects: [],
			accountsettings: [],
			activity: ["types"],
			api: "batchget-topprivatedomain generateJwt get-topprivatedomain groups notification scopeinfo".split(" "),
			apisupportmaps: ["chat"],
			apitotos: [],
			appealform: [],
			axt: [],
			billing: {
				accounts: "bigquery budgetspecs credits currency dimension dimensions export forecast freetrial gcs offers paymentAccounts reopenable referralCode reports signupToken subaccounts state upgrade".split(" "),
				catalog: [
					"commitments",
					"services",
					"skus"
				],
				countries: [],
				coupons: ["instrumentlessAccounts"],
				projects: [],
				"projects:batchGet": [],
				resources: ["projects"],
				"resources:batchGet": [],
				supportedcurrencies: [],
				userfreetrialstatus: [],
				".*": { resources: [] }
			},
			checkdomainadmin: [],
			clouddev: ["create", "redirect"],
			cloudidentity: ["offerOrganizationalAssets"],
			cloudstorage: [
				"b",
				"browser",
				"details",
				"o",
				"storageoptions"
			],
			countries: [],
			country: [],
			crash: ["notificationconfig"],
			crmresources: ["recent"],
			dataprep: ["getAuthUser"],
			datastore: ["settings", "urlsafekey"],
			dm: "composite details list names operation providers types".split(" "),
			domainsList: [],
			emailsettings: [],
			emergencies: [],
			endpoints: [
				"create",
				"customdomains",
				"list",
				"portal"
			],
			folder: ["ancestry"],
			footprints: ["pageview", "search"],
			gae: {
				allows_java: [],
				app: [],
				app_status: [],
				authorizeddomains: [],
				billingstate: [],
				certificate: [],
				certificates: [],
				code_lock: [],
				customdomains: ["create"],
				dashboard: [
					"billing",
					"combined_stats",
					"instances"
				],
				emailsenders: [],
				firewall: ["list", "update"],
				gae_operations: [],
				getappenginebudget: [],
				iap: [],
				instances: [],
				locations: [],
				memcache: ["entry", "flush"],
				mvm_group: [],
				operation_status: [],
				quota: [],
				quota_limit: [],
				search: ["index"],
				service_operations: [],
				services: [],
				settings: [],
				taskqueues: [
					"deletetasks",
					"forcerun",
					"pause",
					"purge",
					"tasks"
				],
				updateappenginebudget: [],
				version_extended: [],
				versions: [],
				versions_traffic_migration: [],
				versions_traffic_split: []
			},
			gcb: [
				"github",
				"oauth_callback",
				"setup_redirect"
			],
			gce: "initWinPasswordReset listNatStatus listRouterStatus nconInstances rdpfile sslCertificates".split(" "),
			gceTest: [],
			gcf: [
				"call",
				"code",
				"get",
				"uploadUrl"
			],
			gcr: "entities hosts images list redirect vulnerabilities".split(" "),
			iam: "getIamPolicies listServiceAccountKeys roles serviceAccountDescriptor setIamPolicies testIamPermissions validate_contact".split(" "),
			iamInvite: [
				"accept",
				"checkRequired",
				"invite"
			],
			inventory: ["priceInfo"],
			jserror: [],
			kms: "cryptoKeys cryptoKeyVersions destroy importJobs projects locations keyRings".split(" "),
			kubernetes: [],
			marketplace: {
				deployment: ["checkQuota", "create"],
				entitlements: ["deployablereportingserviceaccounts", "reportingserviceaccounts"],
				getsupportid: [],
				infoSharing: [],
				lifecycleEventSharing: [],
				procurement: "procurables providers purchaseInfo sso sendgrid-app sendgrid-email".split(" "),
				product: "google-cloud-platform compute-engine click-to-deploy-images wordpress lamp cloud-sql cloud-storage cloud-storage-for-firebase".split(" "),
				proxy: [
					"facets",
					"partners",
					"publishedSolutions",
					"solutions",
					"versions"
				],
				solutionConfig: [],
				trial: [],
				vm: [
					"click-to-deploy-images",
					"wordpress",
					"lamp"
				]
			},
			monitoring: [],
			operations: [],
			organizations: [],
			orgpolicy: ["constraints"],
			ping: ["ping"],
			platform: ["extensions", "route"],
			preferences: [],
			price_list: [],
			project: {
				ancestry: [],
				".*": {
					abusestate: [],
					cloudstorage: "access b browser hmac interop storageoptions".split(" "),
					dataprep: [
						"redirectUrl",
						"shareData",
						"syncAccessGrantedFlag"
					],
					datastore: ["admin", "builtin"],
					drive: ["app", "configure_app"],
					genomics: ["operations"],
					googleappssdk: [
						"app",
						"publish",
						"supporturlvalidate"
					],
					preferences: [],
					pubsub: ["list", "topic"],
					staticmap: ["list", "allowlist"],
					undelete: [],
					".*": ["agent"]
				}
			},
			projectgdprcontacts: [],
			projectidgeneration: [],
			projectidsuggestion: [],
			projectmccacceptance: [],
			projectprecreatecheck: [],
			quotas: ["requestIncrease"],
			recommendations: [],
			search: ["project", "query"],
			services: ["withoutapis"],
			source: [
				"get",
				"list",
				"repos"
			],
			sourceconnect: [
				"authenticate",
				"list",
				"mirror",
				"repos"
			],
			stackdriver: ["access"],
			support: ["pin"],
			tos: ["check", "required"],
			trace: [
				"projectstats",
				"trace",
				"traces"
			],
			tz: ["data"],
			usersettings: [],
			".*": { pageview: [] }
		};
		var m3b = class {
			constructor(a, b, c) {
				this.A = a;
				this.F = b;
				this.state = c;
			}
			startXhr(a) {
				var b;
				var c = Object;
				var d = c.assign;
				var e = a;
				if ((b = a.latencyUntracked) == null) {
					k3b || (k3b = new IP()), b = !k3b.isInTrackingZone() && !a.isPrefetched;
				}
				a = d.call(c, {}, e, {
					latencyUntracked: b,
					url: R1b(a).url
				});
				c = this.state.getPlatformMetadata();
				e = d = undefined;
				if (c !== undefined) {
					d = c.p2Metadata, e = c.platformType;
				}
				var { origin: f, interactionStopwatch: g } = EP(this.state);
				return new l3b(f, a, (k) => {
					this.A.next(k);
				}, (k) => {
					this.F.next(k);
				}, d, e, g);
			}
		};
		var n3b = function(a) {
			var b = new RegExp(a.A.url.replace(/[-\\^$*+?.()|[\]{}]/g, "\\$&").replace("/", ".*") + "([?&]|$)");
			{
				let c = new PerformanceObserver((d) => {
					if (d = d.getEntriesByType("resource").reduce((e, f) => f.YTb || f.startTime < a.performanceStartTimeMs || e && e.startTime < f.startTime ? e : b.test(f.name) ? f : e, undefined)) {
						d.YTb = true;
						a.requestStartMs = a.startTimeMs + (d.startTime - a.performanceStartTimeMs);
						a.H.disconnect();
						a.H = undefined;
					}
				});
				c.observe({ type: "resource" });
				a.H = c;
			}
		};
		var l3b = class extends DP {
			constructor(a, b, c, d, e, f, g) {
				super();
				this.origin = a;
				this.A = b;
				this.emit = c;
				this.I = d;
				this.interactionStopwatch = g;
				this.abandoned = false;
				this.setP2Metadata(e);
				this.setPlatformType(f);
				n3b(this);
			}
			recordChunk(a) {
				var b;
				var c = Object.assign({}, this.A, {
					startTimeMs: this.startTimeMs,
					endTimeMs: BP(this),
					pageView: this.origin.pageView,
					originAccurate: this.origin.accurate,
					metadata: Object.assign({}, {
						p2Metadata: this.getP2Metadata(),
						platformType: this.getPlatformType()
					}, (b = a.serviceMetadata) != null ? b : {}),
					pageHidden: this.pageHidden
				}, a);
				if (this.interactionStopwatch && !this.interactionStopwatch.isStopped()) {
					c.interaction = this.origin.interaction, c.errorResponse && this.interactionStopwatch.addInteractionError(D1b(Object.assign({}, this.A, { status: 0 }), c));
				}
				setTimeout(() => {
					var d = nP([D1b(Object.assign({}, this.A, { status: 0 }), c)]);
					c.qualityErrorWise = d.qualityErrorWise;
					this.I(c);
				}, 1e3);
			}
			recordResponse(a, b) {
				if (!this.isStopped() && (this.F = a, b)) {
					var c;
					if (typeof b.get !== "function") {
						c = new Map(Object.entries(b));
					} else {
						c = b;
					}
					if (b = c.get("server-timing")) {
						a = new Map();
						b = b.replace(/\s/g, "").split(",");
						for (d of b) {
							b = d.split(";");
							for (let e of b.slice(1)) {
								let f = e.split("=");
								if (!(f.length !== 2 || isNaN(Number(f[1])))) {
									a.set(`${b[0]}_${f[0]}`, Number(f[1]));
								}
							}
						}
						var d = a;
					} else d = undefined;
					this.serverTimingsMs = d;
					this.F.requestId = w1b(c, 1);
					d = this.F;
					c = (c = w1b(c, 2)) ? c === "1" : undefined;
					d.requestSampled = c;
				}
			}
			recordAsAbandoned() {
				if (!this.isStopped()) {
					this.abandoned = true;
				}
			}
			stopAndRecord() {
				if (this.F && !this.isStopped()) {
					this.endTimeMs = BP(this);
					if (!this.A.normalizedPath) {
						this.A.normalizedPath = P1b(this.A.url);
					}
					var a = Object.assign({}, this.A, this.F, { serverTimingsMs: this.serverTimingsMs });
					var b = Object.assign({}, a, {
						pageView: this.origin.pageView,
						originAccurate: this.origin.accurate,
						pageHidden: N2b(this),
						abandoned: this.abandoned,
						startTimeMs: this.startTimeMs,
						endTimeMs: this.endTimeMs,
						metadata: {
							p2Metadata: this.getP2Metadata(),
							platformType: this.getPlatformType()
						}
					});
					if (this.interactionStopwatch && !this.interactionStopwatch.isStopped()) {
						b.interaction = this.origin.interaction;
						if (this.F.errorResponse || A1b(this.F)) {
							this.interactionStopwatch.addInteractionError(C1b(a));
						}
						this.interactionStopwatch.addXhrTiming(b.startTimeMs, b.endTimeMs);
						let c = this.F.encryptedTaskInfo;
						if (c && c.length > 0) {
							this.interactionStopwatch.addEncryptedTaskInfo(c);
						}
					}
					setTimeout(() => {
						b.requestStartMs = this.requestStartMs;
						if (this.H) {
							this.H.disconnect();
						}
						var c = nP([C1b(a)]);
						b.qualityErrorWise = c.qualityErrorWise;
						this.emit(b);
					}, 1e3);
				}
			}
			describe() {
				return `xhr ${this.A.method} ${this.A.url} (pageview id: ${this.origin.pageView.id}, pageview path: ${this.origin.pageView.path}, interaction id: ${this.origin.interaction.id})`;
			}
		};
		var Q1b = [];
		var q3b = function(a, b, c, d, e) {
			var f = Date.now();
			return {
				interaction: {
					kind: "onPage",
					id: _.Yn(),
					ave: c,
					customAve: Object.assign({}, d, { eventType: b })
				},
				pageView: a.state.pageView,
				startTimeMs: f,
				endTimeMs: f,
				metadata: e,
				bubbleIndex: 0,
				pageHidden: document.hidden
			};
		};
		var r3b = class {
			constructor(a, b, c, d) {
				this.A = a;
				this.F = b;
				this.H = c;
				this.state = d;
			}
			startOrLookupInteraction(a) {
				var b = this.state.A.get(a);
				if (b) return b;
				b = _.Yn();
				a = {
					id: b,
					event: a,
					stopwatch: p3b(this, b)
				};
				this.state.A.set(a);
				return a;
			}
			logOnPageInteraction(a, b, c, d) {
				d = this.state.currentInteractionId() || d || "";
				if (d = this.state.A.get(d)) {
					d.stopwatch.addVisualElement(a, b, c);
				} else {
					a = q3b(this, b.eventType, a, b, c), this.F.next(a), this.A.next(a);
				}
			}
			logImpression(a, b, c) {
				this.H.next(q3b(this, "impression", a, b, c));
			}
		};
		var s3b = function(a, b) {
			var c = x1b(a.xhrTimings);
			return {
				interaction: {
					id: a.interactionId,
					kind: "onPage",
					ave: b.ave,
					customAve: b.customAve
				},
				pageView: a.pageView,
				bubbleIndex: b.bubbleIndex,
				metadata: b.metadata,
				startTimeMs: a.startTimeMs,
				endTimeMs: a.endTimeMs || a.startTimeMs,
				pageHidden: document.hidden,
				xhrLatencyPartMs: c,
				remainderLatencyPartMs: a.endTimeMs ? a.endTimeMs - a.startTimeMs - c : 0,
				encryptedTaskInfo: a.encryptedTaskInfo
			};
		};
		var o3b = class extends P2b {
			constructor(a, b, c, d, e) {
				super();
				this.interactionId = a;
				this.pageView = b;
				this.X = d;
				this.I = e;
				this.H = 0;
				this.stopped = this.interrupted = false;
				this.A = [];
				this.U = c.subscribe(() => {
					this.interrupted = true;
					this.U.unsubscribe();
				});
			}
			incrementTaskCount() {
				this.H++;
			}
			decrementTaskCount() {
				this.H--;
			}
			hasRemainingTasks() {
				return this.H > 0;
			}
			getTaskCount() {
				return this.H;
			}
			isInterrupted() {
				return this.interrupted;
			}
			addVisualElement(a, b, c) {
				a = {
					ave: a,
					customAve: b,
					metadata: c,
					bubbleIndex: this.A.length
				};
				this.A.push(a);
				this.X(s3b(this, a));
			}
			stopAndRecord() {
				if (!this.stopped) {
					this.stopped = true;
					this.U.unsubscribe();
					this.endTimeMs = BP(this);
					var a = N2b(this);
					if (this.F.length > 0) setTimeout(() => {
						var b = nP(this.F);
						for (let c of this.A) this.I(Object.assign({}, s3b(this, c), b, { pageHidden: a }));
					}, 1e3);
					else for (let b of this.A) this.I(Object.assign({}, s3b(this, b), {
						qualityErrorWise: 1,
						pageHidden: a
					}));
				}
			}
			isStopped() {
				return this.stopped;
			}
			describe() {
				return `on page interaction VEs [${this.A.map((a) => `(id: ${this.interactionId}, ave: ${U1b(a.ave)}, customAve: ${U1b(a.customAve)})`).join(", ")}]`;
			}
		};
		var u3b = class {
			constructor() {
				this.Jb = _.AP.instance;
				this.F = _.zP(this.Jb, "SettledPagePathTracker_pagePathChangesSubject");
				this.A = _.zP(this.Jb, "SettledPagePathTracker_pageLoadedSubject");
			}
		};
		var v3b = function(a, b) {
			var c = Date.now();
			var d = a.state.pageView;
			var e = F1b();
			setTimeout(() => {
				a.F.next({
					pageView: d,
					interaction: b,
					startTimeMs: c,
					endTimeMs: c,
					metadata: {},
					pageHidden: document.hidden
				});
				if (e) {
					a.H.next(Object.assign({}, e, { pageView: d }));
				}
			}, 4e3);
		};
		var y3b = class {
			constructor(a, b, c, d, e) {
				this.F = a;
				this.A = b;
				this.H = c;
				this.bO = d;
				this.state = e;
			}
			startAppLoad(a, b) {
				var c = {
					id: _.Yn(),
					kind: "appLoad"
				};
				this.state.F = c;
				this.state.wN.add(c);
				v3b(this, c);
				return this.state.lastPageLoadStopwatch = new w3b(this.state.pageView, c, (d) => {
					this.A.next(d);
					var e = this.bO;
					d = d.pageView;
					d.pagePathSettled = true;
					e.A.next(d);
				}, a, b);
			}
			startNavigation() {
				x3b(this.state);
				var a = {
					id: _.Yn(),
					kind: "navigation"
				};
				this.state.wN.add(a);
				v3b(this, a);
				return this.state.lastPageLoadStopwatch = new w3b(this.state.pageView, a, (b) => {
					this.A.next(b);
					var c = this.bO;
					b = b.pageView;
					b.pagePathSettled = true;
					c.A.next(b);
				});
			}
		};
		var z3b = function(a, b, c) {
			var d = `${b} ${a.describe()}`;
			var e = b === "started" ? undefined : `${a.phase} ${a.describe()}`;
			a.H.mark(d, e, true, c);
			a.phase = b;
		};
		var A3b = function(a, b) {
			var c = [];
			if ("disabled" === b) return c;
			c.push({
				eventKey: 317,
				startTimeMs: a.startTimeMs,
				endTimeMs: a.endTimeMs,
				resourceSize: b === "enabled" ? 2 : 1
			});
			var d = 0;
			var e = 0;
			for (let [f, g] of a.A) {
				let k = f;
				let p = g;
				let r = CP(a, p.Q6b);
				let v = CP(a, p.I7b);
				let w;
				if (k === "primary") {
					d = v, w = 291;
				} else {
					if (k === "full") {
						e = v, w = 290;
					}
				}
				if (w) {
					c.push({
						eventKey: w,
						startTimeMs: r,
						endTimeMs: v,
						resourceSize: p.k8b
					});
				}
			}
			if ("enabled" === b) {
				d = d || e || a.endTimeMs, e = e || d || a.endTimeMs, e = Math.max(d, e), a.stepTimeMs.initialized = d, a.stepTimeMs["routing-finished"] = e, a.endTimeMs = e, b = a.interaction.kind === "appLoad" && a.stepTimeMs["routing-started"] ? a.performanceStartTimeMs + a.stepTimeMs["routing-started"] - a.startTimeMs : a.performanceStartTimeMs, d = a.performanceStartTimeMs + d - a.startTimeMs, a = a.performanceStartTimeMs + e - a.startTimeMs, performance.measure("route_transition_initial", {
					start: Math.min(b, d),
					end: d,
					detail: "pan-latency-timing"
				}), performance.measure("route_transition", {
					start: Math.min(b, a),
					end: a,
					detail: "pan-latency-timing"
				});
			} else {
				if ("shadow" === b) {
					a.endTimeMs = a.stepTimeMs["routing-finished"] || a.endTimeMs;
				}
			}
			return c;
		};
		var B3b = function(a) {
			var b;
			var c = (((b = performance) == null ? undefined : b.getEntriesByType("navigation")) || [])[0];
			if (c) {
				a.appLoadMetrics = {
					fetchStartTimeMs: CP(a, c.fetchStart),
					requestStartTimeMs: CP(a, c.requestStart),
					responseStartTimeMs: CP(a, c.responseStart),
					responseEndTimeMs: CP(a, c.responseEnd),
					domContentLoadedTimeMs: CP(a, c.domContentLoadedEventEnd)
				};
			}
		};
		var C3b = function(a) {
			var b = [];
			var c;
			var d = (((c = performance) == null ? undefined : c.getEntriesByType("navigation")) || [])[0];
			var e;
			if (!(d == null ? 0 : (e = d.serverTiming) == null ? 0 : e.length)) return b;
			for (let { name: f, duration: g } of d.serverTiming) g > 0 && (c = undefined, f === "gfet4t7" ? c = 315 : f === "l2gfet4t7" && (c = 316), c && b.push({
				eventKey: c,
				startTimeMs: a.startTimeMs,
				endTimeMs: a.startTimeMs + g
			}));
			return b;
		};
		var D3b = function(a, b) {
			return a.jsResourceSize.filter((c) => CP(a, c.timestamp) <= b).reduce((c, d) => {
				c.transferSize += d.transferSize;
				c.decodedBodySize += d.decodedBodySize;
				c.encodedBodySize += d.encodedBodySize;
				return c;
			}, {
				transferSize: 0,
				decodedBodySize: 0,
				encodedBodySize: 0
			});
		};
		var w3b = class extends P2b {
			constructor(a, b, c, d, e) {
				super();
				this.pageView = a;
				this.interaction = b;
				this.emit = c;
				this.H = GP || (GP = new V2b());
				this.stepTimeMs = {};
				this.redirects = [];
				this.jsModuleLoadMetrics = [];
				this.abandoned = this.failed = false;
				this.phase = "started";
				this.A = new Map();
				this.jsResourceSize = [];
				if (d !== undefined) {
					this.startTimeMs = d;
				}
				if (e !== undefined) {
					this.performanceStartTimeMs = e;
				}
				a.timings = {
					startTimeMs: this.startTimeMs,
					lastRedirectTimeMs: this.startTimeMs
				};
				this.setPlatformType(undefined);
				z3b(this, "started");
			}
			isStopped() {
				return this.finished;
			}
			recordStep(a, b = false, c) {
				c = c !== undefined ? c : BP(this);
				z3b(this, a);
				this.stepTimeMs[a] = c;
				if (b && this.interactiveStep === undefined) {
					this.interactiveStep = a;
				}
			}
			recordRedirect(a, b) {
				a = V1b(a);
				b = V1b(b);
				var c = Date.now();
				this.redirects.unshift({
					from: a,
					to: b,
					timeMs: c
				});
				this.pageView.timings.lastRedirectTimeMs = c;
			}
			recordJsModuleLoad(a) {
				this.jsModuleLoadMetrics.push(a);
			}
			recordResourceLoad(a) {
				if (d3b.test(a.name)) {
					this.jsResourceSize.push({
						timestamp: a.startTime || performance.now(),
						transferSize: a.transferSize,
						decodedBodySize: a.decodedBodySize,
						encodedBodySize: a.encodedBodySize
					});
				}
			}
			recordError(a, b, c, d) {
				this.navigationErrorType = a;
				this.pageLoadErrorMessage = b;
				this.pageLoadErrorExperience = c;
				this.pageLoadErrorHttpStatusCode = d;
			}
			setRawPagePath(a) {
				this.rawPagePath = a;
			}
			markAsFailed() {
				this.failed = true;
			}
			markAsAbandoned() {
				this.abandoned = true;
			}
			stopAndRecord(a = "disabled") {
				this.endTimeMs = BP(this);
				var b = A3b(this, a);
				z3b(this, "finished", this.performanceStartTimeMs + this.endTimeMs - this.startTimeMs);
				var c = this.jsModuleLoadMetrics.map((k) => ({
					start: k.startTimeMs,
					end: k.endTimeMs
				}));
				var d = this.xhrTimings;
				var e = this.F;
				if (a !== "disabled") {
					a = (k) => k.end <= this.endTimeMs, c = c.filter(a), d = d.filter(a), e = e.filter((k) => CP(this, k.timestamp || 0) <= this.endTimeMs);
				}
				var f;
				if (((f = this.interaction) == null ? undefined : f.kind) === "appLoad") {
					B3b(this), b.push(...C3b(this));
				}
				var g = {
					pageView: this.pageView,
					rawPagePath: this.rawPagePath || V1b(location.pathname),
					interaction: this.interaction,
					appLoadMetrics: this.appLoadMetrics,
					jsModuleLoadMetrics: this.jsModuleLoadMetrics,
					jsResourceSize: D3b(this, this.endTimeMs),
					jsModuleLatencyPartMs: x1b(c),
					xhrLatencyPartMs: x1b(d),
					remainderLatencyPartMs: Math.max(0, this.endTimeMs - this.startTimeMs - x1b(c.concat(d))),
					startTimeMs: this.startTimeMs,
					stepTimeMs: this.stepTimeMs,
					interactiveStep: this.interactiveStep,
					redirects: this.redirects,
					endTimeMs: this.endTimeMs,
					failed: this.failed,
					abandoned: this.abandoned,
					metadata: {
						p2Metadata: this.getP2Metadata(),
						platformType: this.getPlatformType()
					},
					navigationErrorType: this.navigationErrorType,
					pageLoadErrorType: this.pageLoadErrorType,
					pageLoadErrorMessage: this.pageLoadErrorMessage,
					pageLoadErrorExperience: this.pageLoadErrorExperience,
					pageLoadErrorHttpStatusCode: this.pageLoadErrorHttpStatusCode,
					qualityErrorWise: 1,
					pageHidden: N2b(this),
					encryptedTaskInfo: this.encryptedTaskInfo,
					subMetrics: b
				};
				if (this.navigationErrorType === "generic" || this.pageLoadErrorType === "Ng2GenericError") {
					setTimeout(() => {
						var k;
						var p = e;
						var r = (k = this.navigationErrorType) != null ? k : this.pageLoadErrorType;
						var v = this.pageLoadErrorHttpStatusCode;
						k = y1b(this.pageLoadErrorExperience);
						if (k === 0 && v) {
							k = v ? v >= 500 && v < 600 ? 3 : 1 : 1;
						}
						p = nP(p);
						if (k === 0 && p.qualityErrorWise !== 1) {
							k = 3;
						}
						var w;
						if (((w = p.relevantError) == null ? undefined : w.eventKind) === "xhr") var D = p.relevantError.xhr;
						else {
							let L;
							if (((L = p.relevantError) == null ? undefined : L.eventKind) === "jsError") var G = p.relevantError.jsError;
						}
						r = {
							qualityErrorWise: k,
							relevantError: {
								timestamp: performance.now(),
								eventKind: "pageLoadError",
								pageLoadError: r,
								xhr: D,
								jsError: G
							}
						};
						this.emit(Object.assign({}, g, r));
					}, 1e3);
				} else {
					e.length > 0 ? setTimeout(() => {
						var k = nP(e);
						this.emit(Object.assign({}, g, k));
					}, 1e3) : this.emit(g);
				}
			}
			clearFailure() {
				this.abandoned = this.failed = false;
				this.pageLoadErrorType = this.navigationErrorType = "";
			}
			describe() {
				return this.interaction.kind === "appLoad" ? `app load [id: ${this.pageView.id}]` : `navigation #${this.pageView.sequence} [id: ${this.pageView.id}]`;
			}
			get finished() {
				return this.phase === "finished";
			}
		};
		var E3b = class extends _.Bra {};
		var EP = function(a) {
			var b = W1b();
			if (b == null ? 0 : b.data) {
				var c = b.data.pageView;
				var d = b.data.interaction;
				b = b.data.interactionStopwatch;
				if (c && d) return {
					origin: {
						pageView: c,
						interaction: d,
						accurate: true
					},
					interactionStopwatch: b
				};
			}
			b = _.aP() ? sharedHostData.getCurrentZone() : Zone.current;
			c = b.get("pageView");
			d = b.get("interaction");
			b = b.get("interactionStopwatch");
			if (c && d) return {
				origin: {
					pageView: c,
					interaction: d,
					accurate: true
				},
				interactionStopwatch: b
			};
			c = F3b;
			d = undefined;
			if (a.lastPageLoadStopwatch && !a.lastPageLoadStopwatch.isStopped()) {
				c = a.lastPageLoadStopwatch.interaction, d = a.lastPageLoadStopwatch;
			} else {
				if (a.wN.getLast()) {
					c = a.wN.getLast(), (b = a.A.get(c.id)) && !b.stopwatch.isStopped() && (d = b.stopwatch);
				}
			}
			return {
				origin: {
					pageView: a.pageView,
					interaction: c,
					accurate: false
				},
				interactionStopwatch: d
			};
		};
		var x3b = function(a) {
			var b = a.pageView;
			a.pageView = X1b(a.pageView.sequence + 1, a.pageView.id);
			a.ea.next({
				previous: b,
				current: a.pageView
			});
		};
		var H3b = class {
			constructor() {
				this.Jb = _.AP.instance;
				this.fa = new _.yP(qP`pageView`, X1b(0));
				this.aa = new _.yP(qP`navigationStateProvider`);
				this.ea = _.zP(this.Jb, qP`pageViewChanges`);
				this.pageViewChanges = this.ea.asObservable();
				this.X = new _.yP(qP`lastPageLoadStopwatch`);
				this.I = new _.yP(qP`appLoadInteraction`);
				this.ta = new _.yP(qP`recentInteractions`, new E3b(5));
				this.ma = new _.yP(qP`recentNonErrorCanonicalPath`);
				this.oa = new _.yP(qP`interactionRegistry`, new G3b());
				this.na = new _.yP(qP`rifMetadataProvider`);
				this.U = new _.yP(qP`lastOnpageInteractionId`);
				this.R = new _.yP(qP`lastOnpageInteractionIdBeforeNavigation`);
			}
			get pageView() {
				return this.fa.get();
			}
			set pageView(a) {
				this.fa.set(a);
			}
			get navigationStateProvider() {
				return this.aa.get();
			}
			set navigationStateProvider(a) {
				if (a) {
					this.aa.set(a);
				}
			}
			get lastPageLoadStopwatch() {
				return this.X.get();
			}
			set lastPageLoadStopwatch(a) {
				if (a) {
					this.X.set(a);
				}
			}
			get F() {
				return this.I.get();
			}
			set F(a) {
				if (a) {
					this.I.set(a);
				}
			}
			get wN() {
				return this.ta.get();
			}
			get H() {
				return this.ma.get();
			}
			set H(a) {
				if (a) {
					this.ma.set(a);
				}
			}
			get A() {
				return this.oa.get();
			}
			get rifMetadataProvider() {
				return this.na.get();
			}
			set rifMetadataProvider(a) {
				if (a) {
					this.na.set(a);
				}
			}
			get lastOnpageInteractionId() {
				return this.U.get();
			}
			set lastOnpageInteractionId(a) {
				if (a) {
					this.U.set(a);
				}
			}
			get lastOnpageInteractionIdBeforeNavigation() {
				return this.R.get();
			}
			set lastOnpageInteractionIdBeforeNavigation(a) {
				if (a) {
					this.R.set(a);
				}
			}
			currentInteractionId() {
				var a;
				var b;
				var c;
				var d;
				return (d = (a = W1b()) == null ? undefined : (b = a.data) == null ? undefined : (c = b.interaction) == null ? undefined : c.id) != null ? d : "";
			}
			getPlatformMetadata({ Ryb: a } = {}) {
				if (this.rifMetadataProvider) {
					var b = this.rifMetadataProvider.getCurrentlyRunningSandboxId();
					if (b) return {
						p2Metadata: this.rifMetadataProvider.getRifMetadata(b),
						platformType: 5
					};
				}
				b = W1b();
				if (b == null ? 0 : b.data) {
					let c = b.data.p2Metadata;
					if (c) return {
						p2Metadata: c,
						platformType: b.data.platformType
					};
				}
				if (a && this.lastPageLoadStopwatch) return {
					p2Metadata: this.lastPageLoadStopwatch.getP2Metadata(),
					platformType: this.lastPageLoadStopwatch.getPlatformType()
				};
			}
		};
		var G3b = class {
			constructor() {
				this.A = new Map();
				this.ZR = new Map();
			}
			get(a) {
				return (a = a instanceof Event ? this.A.get(a) : a) ? this.ZR.get(a) || null : null;
			}
			set(a) {
				this.A.set(a.event, a.id);
				this.ZR.set(a.id, a);
			}
			delete(a) {
				if (a = this.ZR.get(a)) {
					this.A.delete(a.event);
					this.ZR.delete(a.id);
				}
			}
			count() {
				return this.ZR.size;
			}
		};
		var Y1b;
		var Z1b = class {
			constructor() {
				this.Jb = _.AP.instance;
				this.S6a = _.zP(this.Jb, sP`pageLoadStartSubject`);
				this.T6a = _.zP(this.Jb, sP`pageLoadSubject`);
				this.FXa = _.zP(this.Jb, sP`customEventSubject`);
				this.veb = new M2b();
				this.CVb = (() => {
					var a = _.zP(this.Jb, sP`xhrSubject`);
					a.subscribe((b) => {
						this.veb.next(b);
					});
					return a;
				})();
				this.ueb = new M2b();
				this.BVb = (() => {
					var a = _.zP(this.Jb, sP`xhrChunkSubject`);
					a.subscribe((b) => {
						this.ueb.next(b);
					});
					return a;
				})();
				this.ATa = _.zP(this.Jb, sP`adHocErrorSubject`);
				this.zTa = _.zP(this.Jb, sP`adHocClientErrorSubject`);
				this.N3a = _.zP(this.Jb, sP`jsErrorSubject`);
				this.K5a = _.zP(this.Jb, sP`networkInformationPageLoadSubject`);
				this.p6a = _.zP(this.Jb, sP`onPageInteractionStartEventSubject`);
				this.o6a = _.zP(this.Jb, sP`onPageInteractionEventSubject`);
				this.J1a = _.zP(this.Jb, sP`impressionEventSubject`);
				this.O3a = _.zP(this.Jb, sP`jsModuleLoadSubject`);
				this.bO = new u3b();
				this.D6a = _.zP(this.Jb, sP`optimisticCacheEntrySubject`);
				this.pageLoadStartEntries = this.S6a.asObservable();
				this.pageLoadEntries = this.T6a.asObservable();
				this.customEventEntries = this.FXa.asObservable();
				this.xhrEntries = this.veb.R.pipe(t3b(this.bO));
				this.xhrChunkEntries = this.ueb.R.pipe(t3b(this.bO));
				this.onPageInteractionStartEntries = this.p6a.asObservable();
				this.onPageInteractionEntries = this.o6a.asObservable();
				this.impressionEntries = this.J1a.asObservable();
				this.adHocErrorEntries = this.ATa.asObservable();
				this.adHocClientErrorEntries = this.zTa.asObservable();
				this.jsErrorEntries = this.N3a.asObservable().pipe(t3b(this.bO));
				this.networkStateEntries = _.Ff(G1b().pipe(_.uf((a) => Object.assign({}, a, { pageView: this.currentPageView }))), this.K5a.asObservable());
				this.jsModuleLoadEntries = this.O3a.asObservable();
				this.optimisticCacheEntries = this.D6a.asObservable();
				this.state = new H3b();
				this.YGa = new y3b(this.S6a, this.T6a, this.K5a, this.bO, this.state);
				this.cGa = new r3b(this.o6a, this.p6a, this.J1a, this.state);
				this.DVb = new m3b(this.CVb, this.BVb, this.state);
				this.TIb = new $2b(this.N3a, this.state);
				this.Xxa = new Y2b(this.FXa, this.state);
				this.P3a = new c3b(this.O3a, this.state);
			}
			get lastPageLoadStopwatch() {
				return this.state.lastPageLoadStopwatch;
			}
			get rifMetadataProvider() {
				return this.state.rifMetadataProvider;
			}
			setRifMetadataProvider(a) {
				this.state.rifMetadataProvider = a;
			}
			get navigationStateProvider() {
				return this.state.navigationStateProvider;
			}
			get lastOnpageInteractionIdBeforeNavigation() {
				return this.state.lastOnpageInteractionIdBeforeNavigation;
			}
			get lastOnpageInteractionId() {
				return this.state.lastOnpageInteractionId;
			}
			set lastOnpageInteractionIdBeforeNavigation(a) {
				this.state.lastOnpageInteractionIdBeforeNavigation = a;
			}
			setNavigationStateProvider(a) {
				this.state.navigationStateProvider = a;
			}
			get currentPageView() {
				return this.state.pageView;
			}
			get pageViewChanges() {
				return this.state.pageViewChanges;
			}
			get wN() {
				return this.state.wN;
			}
			get warmLoading() {
				return this.state.lastPageLoadStopwatch === undefined ? false : this.state.lastPageLoadStopwatch.interaction.kind === "navigation";
			}
			startAppLoad(a, b) {
				return this.YGa.startAppLoad(a, b);
			}
			startNavigation() {
				return this.YGa.startNavigation();
			}
			startCustomEvent(a, b = 0) {
				return this.Xxa.startCustomEvent(a, b);
			}
			startGenericStopwatch() {
				return new O2b();
			}
			logCustomEvent(a, b, c) {
				this.Xxa.logCustomEvent(a, b, undefined, c);
			}
			logLongTask(a, b) {
				this.Xxa.logCustomEvent("long_task", a, b);
			}
			startXhr(a) {
				return this.DVb.startXhr(a);
			}
			startOrLookupInteraction(a) {
				return this.cGa.startOrLookupInteraction(a);
			}
			getInteraction(a) {
				return this.state.A.get(a);
			}
			cleanupInteraction(a) {
				this.state.A.delete(a);
			}
			numberOfActiveInteractions() {
				return this.state.A.count();
			}
			logOnPageInteraction(a, b, c, d) {
				this.cGa.logOnPageInteraction(a, b, c, d);
			}
			logImpression(a, b, c) {
				this.cGa.logImpression(a, b, c);
			}
			logAdHocError(a) {
				var b = this.currentPageView;
				setTimeout(() => {
					this.ATa.next(Object.assign({}, a, { pageView: b }));
				}, 1e3);
			}
			logClientError(a) {
				var { origin: b } = EP(this.state);
				var c = b.pageView;
				b = b.interaction.id;
				var d = Date.now();
				a = Object.assign({}, a, {
					pageView: c,
					interactionId: b,
					logTimeMs: d
				});
				this.zTa.next(a);
			}
			logJsError(a, b = this.state.getPlatformMetadata()) {
				this.TIb.logJsError(a, b);
			}
			logOptimisticCacheUsage(a) {
				var b;
				this.D6a.next(Object.assign({}, a, {
					pageView: this.currentPageView,
					interaction: this.state.F,
					navigationState: (b = this.state.navigationStateProvider) == null ? undefined : b.getNavigationState()
				}));
			}
			startJsModuleLoad(a) {
				return this.P3a.startJsModuleLoad(a);
			}
			getRecentInteractions() {
				return this.wN.getValues().reverse();
			}
			getRecentNonErrorCanonicalPath() {
				return this.state.H;
			}
			handleCanonicalPathResolveEvent(a) {
				this.currentPageView.path = a;
				this.bO.F.next(this.currentPageView);
				if (!a.startsWith("/navigation-error")) {
					this.state.H = a;
				}
			}
			getPlatformMetadata(a) {
				return this.state.getPlatformMetadata({ Ryb: a });
			}
			currentInteractionId() {
				return this.state.currentInteractionId();
			}
			logHandledJsException(a) {
				var b = a.error;
				if (typeof b === "object") {
					b[Z2b] = a.errorExperience;
				}
				_.Fra(a.error.message, a.error, "unknown");
			}
			handlePerformanceObserverResourceTiming(a) {
				var b = this.YGa;
				if (b.state.lastPageLoadStopwatch && !b.state.lastPageLoadStopwatch.isStopped()) {
					b.state.lastPageLoadStopwatch.recordResourceLoad(a);
				}
				b = this.P3a;
				if (d3b.test(a.name)) {
					b.F.next(a);
				}
			}
		};
		var tP = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var I3b = _.bd(tP);
		var J3b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getPlatformType() {
				return _.Lm(this, 1);
			}
			setPlatformType(a) {
				return _.cn(this, 1, a);
			}
		};
		var K3b = class extends _.h {
			constructor(a) {
				super(a);
			}
			setRawPagePath(a) {
				return _.Lj(this, 5, a);
			}
			getPlatformMetadata() {
				return _.Z(this, J3b, 10);
			}
			getNavigationState() {
				return _.l(this, 27);
			}
		};
		K3b.prototype.I = "zSmm2";
		var e2b = new _.Wt(108, _.sJ, K3b);
		_.JP = class {
			constructor(a, b) {
				var c = b.path();
				b.gN((e) => {
					if (!$1b(c)) {
						$1b(e);
					}
				});
				a = _.aP() ? a.ref.parent : a.ref;
				b = _.bP(a);
				_.l(b, 1);
				this.Mla = _.l(b, 14) || "global";
				_.uj(b, 15, _.oj());
				this.Noa = s2b(_.bP(a));
				this.rna = q2b(_.bP(a));
				this.flags = a.pantheon_flags_init_args || {};
				var d;
				this.A = (d = a.pantheon_account_chooser_data) != null ? d : null;
				this.referrer = _.l(b, 8) || document.referrer;
				_.uj(b, 10, _.oj());
				_.l(b, 7);
				r2b(_.bP(a));
				this.F = _.vc(b, 18, _.Db, _.oj());
				this.Fga = _.l(b, 32) || "";
				_.Z(b, l2b, 21);
				if (_.sn(b, _.qJ, 29)) {
					_.Z(b, _.qJ, 29);
				}
				if (!u2b(_.bP(a))) {
					new n2b();
				}
				_.l(b, 33);
				_.l(b, 12);
				_.l(b, 2);
				_.l(b, 16);
				this.country = _.l(b, 19);
				_.uj(b, 25, _.oj());
				if (!t2b(_.bP(a))) {
					new o2b();
				}
			}
		};
		_.JP.J = function(a) {
			return new (a || _.JP)(_.ae(_.fP), _.ae(_.gm));
		};
		_.JP.sa = _.Cd({
			token: _.JP,
			factory: _.JP.J,
			wa: "root"
		});
		var c2b = { [3]: "click" };
		var f2b = new Set([
			3,
			30,
			37,
			15
		]);
		var N3b = class extends _.yJ {
			constructor() {
				var a = new L3b();
				super(new M3b(), null, a, {
					OCb: false,
					yga: undefined,
					TLa: false
				});
				this.oa = a;
				this.oa.vba(false);
				this.oa.CJa(true);
				this.oa.BJa(true);
			}
			vba(a) {
				this.oa.vba(a);
			}
		};
		var O3b = function(a, b) {
			var c = Number(b);
			if (Object.values(w2b).includes(c)) {
				a.addMetadataHandler((d) => {
					if (d instanceof uP) {
						_.cn(d, 2, c);
					}
				});
			}
		};
		var P3b = function(a, b) {
			a.addMetadataHandler((c) => {
				if (c instanceof uP) {
					_.Lj(c, 1, b);
				}
				if (c instanceof _.sJ) {
					c = a.H(c), c = a.X(c), _.Lj(c, 1, b);
				}
			});
		};
		var Q3b = function(a, b) {
			a.addMetadataHandler((c) => {
				if (c instanceof uP && !_.zn(c, 6)) {
					_.Lj(c, 6, b);
				}
				if (c instanceof _.sJ) {
					c = a.H(c), c = a.X(c), _.Lj(c, 2, b);
				}
			});
		};
		var R3b = function(a, b) {
			a.addMetadataHandler((c) => {
				if (c instanceof _.sJ) {
					_.Lj(c, 4, b);
				}
			});
		};
		var S3b = function(a, b) {
			a.addMetadataHandler((c) => {
				if (c instanceof _.sJ) {
					_.Lj(c, 3, b);
				}
			});
		};
		var T3b = function(a, b) {
			a.addMetadataHandler((c) => {
				if (c instanceof _.sJ) {
					c.DA(b);
				}
			});
		};
		var U3b = function(a, b) {
			a.addMetadataHandler((c) => {
				if (c instanceof _.sJ) {
					c = a.H(c), _.ln(c, tP, 58, b);
				}
			});
		};
		var V3b = class extends _.jz {
			constructor(a) {
				super(a);
			}
			X(a) {
				var b = _.Z(a, tP, 58);
				if (!b) {
					b = new tP(), _.ln(a, tP, 58, b);
				}
				return b;
			}
			H(a) {
				var b = _.uQa(a, e2b);
				if (!b) {
					b = new K3b(), _.xt(a, e2b, b);
				}
				return b;
			}
		};
		var W3b = class {
			Hha(a) {
				return new V3b(a);
			}
			ega(a, b, c) {
				b = b.trim();
				c = c.trim();
				switch (Number(b)) {
					case 2:
						O3b(a, c);
						break;
					case 1:
						P3b(a, c);
						break;
					case 6:
						Q3b(a, c);
						break;
					case 32:
						R3b(a, c);
						break;
					case 31:
						S3b(a, c);
						break;
					case 33:
						T3b(a, v2b(c));
						break;
					case 34: U3b(a, I3b(c));
				}
			}
			DA() {}
		};
		var M3b = class {
			F() {
				return new _.Qv();
			}
			I() {
				return new W3b();
			}
			H() {
				return new uP();
			}
		};
		var L3b = class {
			vba() {}
			CJa() {}
			BJa() {}
			dispatch() {}
			flush() {}
		};
		var X3b = class extends N3b {
			constructor() {
				var a = _.rP();
				super();
				this.Nc = a;
			}
			za(a, b, c) {
				c = c.map((e) => e.getTag().getID());
				var d = b2b(b.getElement());
				g2b(this.Nc, a, b.getTag(), c, d);
				if (a != null) {
					_.uzb(this, new _.Zyb(a));
				}
			}
			log() {}
			Hr(a, b, c) {
				c2b[b] = a;
				super.Hr(a, b, c);
			}
		};
		_.KP = class extends _.Jy {
			constructor() {
				super();
				this.A = null;
				if (_.KP.instance) return _.KP.instance;
				_.KP.instance = this;
			}
			F() {
				if (this.A) return this.A;
				this.A = new X3b();
				_.szb(this.A);
				this.A.Hr("hover", 9);
				this.A.Hr("scroll", 22);
				this.A.Hr("drag", 30);
				this.A.Hr("input", 15);
				return this.A;
			}
		};
		_.KP.logger = null;
		_.KP.J = function(a) {
			return new (a || _.KP)();
		};
		_.KP.sa = _.Cd({
			token: _.KP,
			factory: _.KP.J,
			wa: "root"
		});
		[
			0,
			43,
			2
		].concat([
			1,
			28,
			30
		]);
		var LP = class {
			constructor(a, b, c) {
				this.resourceType = a;
				this.iconName = b;
				this.nZ = c;
			}
		};
		var Y3b = new Map([
			[0, new LP(0, "project", "iam")],
			[43, new LP(43, "folder", "iam")],
			[2, new LP(2, "domain", "iam")],
			[28, new LP(28, "bucket", "shell")],
			[30, new LP(30, "service-accounts", "iam")]
		]);
		Y3b.get(0);
		Y3b.get(43);
		Y3b.get(2);
		Array.from(new Map([["ACTIVE", 1], ["DELETE_REQUESTED", 2]]).entries()).map(([a, b]) => [b, a]);
		_.Z3b = class extends _.h {
			constructor(a) {
				super(a, 0, _.Z3b.messageId);
			}
			getEmail() {
				return _.ek(this, 4, _.gb);
			}
			getDisplayName() {
				return _.ek(this, 9, _.gb);
			}
		};
		_.Z3b.messageId = "p6n.a";
		_.$3b = new _.hP("45407939");
		_.a4b = new _.hP("45405970");
		var b4b;
		var c4b = class {
			constructor() {
				_.m(_.JP);
				var a = [
					"pantheon",
					"images",
					"account_circle_filled_background.svg"
				];
				if ((_.u1b() || _.cP(_.$3b)) && (a[0] === "" && a[1] === "pantheon" && a[2] === "images" || a[0] === "pantheon" && a[1] === "images")) {
					a.splice(a[0] === "" ? 2 : 1, 0, "tpc");
				}
				var b = a.reduce;
				if (!b4b) {
					var c = t1b();
					c = _.bP(c);
					c = _.Z(c, p2b, 26) || new p2b();
					c = _.l(c, 2);
					if (c === "" || c === "static-tpclp.goog") c = "www.gstatic.com";
					b4b = _.dd(`https://${c}`);
				}
				c = b4b;
				b.call(a, _.zqa, c);
			}
		};
		_.MP = class extends _.h {
			constructor(a) {
				super(a, 0, _.MP.messageId);
			}
		};
		_.d4b = _.bd(_.MP);
		_.MP.messageId = "p6n.p";
		_.e4b = new _.he("SessionManagementClient", {
			wa: "root",
			factory: () => new c4b()
		});
		h2b.prototype.next = function() {
			return f4b;
		};
		h2b.prototype.f3 = function() {
			return this;
		};
		var NP = class {
			constructor(a) {
				this.A = a;
			}
			f3() {
				return new g4b(this.A());
			}
			[Symbol.iterator]() {
				return new OP(this.A());
			}
			F() {
				return new OP(this.A());
			}
		};
		var g4b = class extends h2b {
			constructor(a) {
				super();
				this.A = a;
			}
			next() {
				return this.A.next();
			}
			[Symbol.iterator]() {
				return new OP(this.A);
			}
			F() {
				return new OP(this.A);
			}
		};
		var OP = class extends NP {
			constructor(a) {
				super(() => a);
				this.H = a;
			}
			next() {
				return this.H.next();
			}
		};
		var PP = function() {};
		_.Fs(PP, i2b);
		PP.prototype.Dn = function() {
			var a = 0;
			for (let b of this) a++;
			return a;
		};
		PP.prototype[Symbol.iterator] = function() {
			return h4b(this.f3(true)).F();
		};
		PP.prototype.clear = function() {
			var a = Array.from(this);
			for (let b of a) this.remove(b);
		};
		_.Fs(j2b, PP);
		_.aa = j2b.prototype;
		_.aa.set = function(a, b) {
			QP(this);
			try {
				this.A.setItem(a, b);
			} catch (c) {
				if (this.A.length == 0) throw "Storage mechanism: Storage disabled";
				throw "Storage mechanism: Quota exceeded";
			}
		};
		_.aa.get = function(a) {
			QP(this);
			a = this.A.getItem(a);
			if (typeof a !== "string" && a !== null) throw "Storage mechanism: Invalid value was encountered";
			return a;
		};
		_.aa.remove = function(a) {
			QP(this);
			this.A.removeItem(a);
		};
		_.aa.Dn = function() {
			QP(this);
			return this.A.length;
		};
		_.aa.f3 = function(a) {
			QP(this);
			var b = 0;
			var c = this.A;
			var d = new h2b();
			d.next = function() {
				if (b >= c.length) return f4b;
				var e = c.key(b++);
				if (a) return {
					value: e,
					done: false
				};
				e = c.getItem(e);
				if (typeof e !== "string") throw "Storage mechanism: Invalid value was encountered";
				return {
					value: e,
					done: false
				};
			};
			return d;
		};
		_.aa.clear = function() {
			QP(this);
			this.A.clear();
		};
		_.aa.key = function(a) {
			QP(this);
			return this.A.key(a);
		};
		_.Fs(k2b, j2b);
		i4b = new _.he("Session Storage", {
			wa: "root",
			factory: () => new k2b()
		});
		_.RP = class {
			constructor() {
				this.sessionStorage = _.m(i4b);
				this.A = _.m(_.eP);
				this.H = new _.yP("clientSessionIdSharedStateId");
				this.F = new _.yP("crossAppClientSessionIdSharedStateId");
				var a = this.sessionStorage.get("pantheonSessionId");
				if (a) {
					this.J4 = a;
				} else {
					this.J4 = _.Yn(), this.sessionStorage.set("pantheonSessionId", this.J4);
				}
				if (this.H.get() === undefined) {
					this.H.set(_.Yn());
				}
				if (this.F.get() === undefined) {
					a = this.F;
					var b = a.set;
					a: {
						let f = this.sessionStorage.get("crossAppClientSessionId");
						if (f) {
							var c;
							if ((c = this.A.history) == null ? 0 : c.state) {
								var d = f;
								break a;
							}
							var e;
							c = (e = this.A.document) == null ? undefined : e.referrer;
							let g;
							e = (g = this.A.location) == null ? undefined : g.href;
							if (c && e) {
								let k = new URL(c);
								let p = new URL(e);
								if (k.host === p.host && k.port === p.port && k.protocol === p.protocol) {
									d = f;
									break a;
								}
							}
						}
						f = _.Yn();
						this.sessionStorage.set("crossAppClientSessionId", f);
						d = f;
					}
					b.call(a, d);
				}
			}
			IY() {
				return this.J4;
			}
		};
		_.RP.J = function(a) {
			return new (a || _.RP)();
		};
		_.RP.sa = _.Cd({
			token: _.RP,
			factory: _.RP.J,
			wa: "root"
		});
		j4b = document.createElementNS("http://www.w3.org/2000/svg", "svg");
		_.SP = class {
			constructor() {
				this.icon = j4b;
				this.bj = _.m(_.Jf).nativeElement;
			}
			Wb() {
				var a = document.importNode(this.icon, true);
				a.setAttribute("aria-hidden", "true");
				if (this.bj.firstChild) {
					this.bj.replaceChild(a, this.bj.firstChild);
				} else {
					this.bj.appendChild(a);
				}
			}
		};
		_.SP.J = function(a) {
			return new (a || _.SP)();
		};
		_.SP.ka = _.u({
			type: _.SP,
			da: [["cm-icon"]],
			inputs: { icon: "icon" },
			features: [_.su],
			ha: 0,
			ia: 0,
			template: function() {},
			styles: ["[_nghost-%COMP%]{display:inline-block;line-height:0;vertical-align:middle;fill:currentColor;--cm-comp-icon-size:unset}[_nghost-%COMP%]     svg{forced-color-adjust:auto;width:var(--cm-comp-icon-size);height:var(--cm-comp-icon-size)}.cm-icon--vertical-align-cap-middle[_nghost-%COMP%]{margin-top:calc((var(--cm-icon-cap-height, .712em) - var(--cm-icon-x-height, 1ex))/-2)}.cm-icon--legacy-margins[_nghost-%COMP%]{margin-top:-2px}"]
		});
		_.k4b = (0, _.mP)`<svg data-icon-name="externalLinkIcon" viewBox="0 0 18 18" width="18" height="18"><path fill-rule="evenodd" d="M13.85 5H14V4h-4v1h2.15l-5.36 5.364.848.848L13 5.85V8h1V4h-1v.15l.15-.15.85.85zM8 4H4.995A1 1 0 004 4.995v8.01a1 1 0 00.995.995h8.01a1 1 0 00.995-.995V10h-1v3H5V5h3z"/></svg>`.firstElementChild;
		_.m4b = function() {
			var a = [(0, _.Ij)`aria-`];
			return (b, c, d) => {
				_.vea(a, b, c, d);
			};
		};
		_.mvb.prototype.ig = _.ca(176, function() {
			return _.Lm(this, 1);
		});
		_.n4b = function(a) {
			new Promise((b) => {
				a.fa(() => b(_.HA(a)));
			});
		};
		_.o4b = function(a, b) {
			return b ? _.Mv(a, function(c) {
				return !b || typeof c.className === "string" && _.Ba(c.className.split(/\s+/), b);
			}) : null;
		};
		_.p4b = new _.he("deferredTypes");
		new _.he("hadPreloadClick");
		var TP;
		var UP;
		var s4b;
		var XP;
		var w4b;
		var z4b;
		var F4b;
		var E4b;
		var bQ;
		var K4b;
		var S4b;
		var U4b;
		var W4b;
		var Y4b;
		var cQ;
		var eQ;
		var a5b;
		var c5b;
		var e5b;
		var fQ;
		var g5b;
		var q5b;
		var r5b;
		var I5b;
		var H5b;
		var J5b;
		var K5b;
		var N5b;
		var O5b;
		var P5b;
		var Q5b;
		var F5b;
		var G5b;
		var S5b;
		var T5b;
		var m5b;
		var D4b;
		var kQ;
		var W5b;
		var Y5b;
		var Z5b;
		TP = function() {
			throw Error(_.fba(_.eba));
		};
		UP = function(a) {
			var b = _.eba;
			if (!a) throw Error(_.fba(b) || String(a));
		};
		_.VP = function(a) {
			return _.jb(a, _.Js);
		};
		q4b = function(a) {
			return _.kb((b) => b instanceof a);
		};
		_.r4b = function(...a) {
			return _.kb((b) => a.some((c) => c(b)));
		};
		s4b = function(a, b = false) {
			var c = typeof a;
			if (a == null) return a;
			if (c === "bigint") return String((0, _.Sb)(64, a));
			if (_.yb(a)) return c === "string" ? _.Rb(a) : b ? _.Aba(a) : _.Qb(a);
		};
		t4b = function(a) {
			if (!((a == null ? undefined : a.prototype) instanceof _.h)) throw Error();
			return a[_.Yb] || (a[_.Yb] = _.Zb(a));
		};
		XP = function() {
			if (_.WP !== _.WP) throw Error();
		};
		_.YP = function(a, b, c, d, ...e) {
			var f = new u4b(a, b, c, d, e);
			if (a) {
				a[_.IQa] != null || (a[_.IQa] = f);
			}
			return () => f;
		};
		_.ZP = function(a, b) {
			return (() => {
				var c = new v4b(a, b);
				return () => c;
			})();
		};
		w4b = function(a) {
			if (a.indexOf(".") === 0) {
				a = a.substring(1);
			}
			return _.JQa.get(a);
		};
		aQ = function(a) {
			var b;
			if ((b = a[$P]) != null) {
				a = b;
			} else {
				XP(), a = a[$P] = x4b(a.A);
			}
			return a;
		};
		z4b = function(a) {
			var b;
			return (b = a[$P]) != null ? b : a[$P] = y4b(a.F(_.WP));
		};
		F4b = function(a, b, c) {
			if (c) {
				var d;
				var e = (d = a[A4b]) != null ? d : a[A4b] = new Map();
			} else if (b) {
				let f;
				e = (f = a[B4b]) != null ? f : a[B4b] = new Map();
			} else {
				let f;
				e = (f = a[C4b]) != null ? f : a[C4b] = new Map();
			}
			a = D4b(a);
			if (e.size !== a.size) for (let f of a.values()) a = f.A(_.WP), d = z4b(f).getName(), e.set(E4b(a, d, b, c), f);
			return e;
		};
		E4b = function(a, b, c, d) {
			return d ? b : c ? bQ(b) : a == null ? `[${b}]` : `[${a}.${b}]`;
		};
		H4b = function(a) {
			if (a.length === 2) {
				a = a[1], a = a > "z" || a < "a" ? a : G4b(a.charCodeAt(0) - 32);
			} else {
				a = "";
			}
			return a;
		};
		bQ = function(a) {
			return a.replace(I4b, H4b);
		};
		K4b = function(a) {
			return a.replace(J4b, (b) => `_${b > "Z" || b < "A" ? b : G4b(b.charCodeAt(0) + 32)}`);
		};
		_.O4b = function(a) {
			var b;
			return (b = a[L4b]) != null ? b : a[L4b] = new Map(M4b(aQ(a)).map((c) => [c.getName(), N4b(c)]));
		};
		_.Q4b = function(a) {
			var b;
			return (b = a[P4b]) != null ? b : a[P4b] = new Map(M4b(aQ(a)).map((c) => [N4b(c), c.getName()]));
		};
		S4b = function(a) {
			return !!(_.ec(R4b) && _.ec(R4b) in a);
		};
		U4b = function(a) {
			return !!(_.ec(T4b) && _.ec(T4b) in a);
		};
		W4b = function(a) {
			return !!(_.ec(V4b) && _.ec(V4b) in a);
		};
		Y4b = function(a) {
			return !!(_.ec(X4b) && _.ec(X4b) in a);
		};
		cQ = function(a) {
			return !!(_.ec(Z4b) && _.ec(Z4b) in a);
		};
		eQ = function(a) {
			return !!(_.ec(dQ) && _.ec(dQ) in a);
		};
		a5b = function(a, b) {
			return b ? U4b(a) && !$4b.has(b) : false;
		};
		c5b = function(a) {
			return !!(_.ec(b5b) && _.ec(b5b) in a);
		};
		e5b = function(a) {
			return !!(_.ec(d5b) && _.ec(d5b) in a);
		};
		fQ = function(a) {
			return !!(_.ec(f5b) && _.ec(f5b) in a);
		};
		iQ = function(a, b, c, d) {
			if (_.ec(_.gQ.get) && c === null && b.typeName === "google.protobuf.Value" && !eQ(a)) return _.rp(_.ro(new _.qo(), 0));
			if (c != null) {
				var e = b.messageType;
				var f = b.isRepeated;
				var g = b.enumType;
				var k = b.HWa;
				if (b.isMap) {
					if (fQ(a) && Array.isArray(c)) {
						c = Object.fromEntries(c.map((r) => [_.jb(r.key, _.Js), _.jb(r.value, _.Js)]));
					}
					b = e[1];
					var p = e[2];
					e = new Map();
					for (let [r, v] of Object.entries(_.jb(c, hQ))) {
						d = r;
						f = v;
						switch (b.fieldType) {
							case 8:
								switch (d) {
									case "true":
										c = true;
										break;
									case "false":
										c = false;
										break;
									default: continue;
								}
								break;
							case 5:
							case 13:
							case 17:
							case 7:
							case 15:
								c = Number(_.jb(d, _.Js));
								if (!Number.isFinite(c)) continue;
								break;
							case 2:
							case 1:
								c = Number(_.jb(d, _.Js));
								if (Number.isNaN(c) && d !== "NaN") continue;
								break;
							case 14:
								isNaN(d) || (d = +d);
								c = iQ(a, b, _.jb(d, _.Js));
								break;
							default: c = iQ(a, b, _.jb(d, _.Js));
						}
						d = iQ(a, p, _.jb(f, _.Js));
						if (d != null) {
							e.set(c, d);
						}
					}
					return e;
				}
				if (f && !d) {
					e = [];
					for (p of _.jb(c, _.zPa)) UP(p != null), c = iQ(a, b, p, true), c != null && e.push(c);
					return e;
				}
				if (e) return g5b(a, e, c);
				if (g) return typeof c !== "string" ? _.VP(_.jb(k, _.Js)(c, 1), `incompatible wire value for field ${b.field.getName()}: ${c}`) : _.O4b(g).get(_.jb(c, _.mb));
				if (_.ec(jQ) && _.ec(jQ) in a && typeof c === "number" && !Number.isFinite(c)) {
					c = String(c);
				}
				return _.VP(_.jb(k, _.Js)(c, 1), `incompatible wire value for field ${b.field.getName()}: ${c}`);
			}
		};
		g5b = function(a, b, c) {
			if (c != null) {
				var d = c[h5b];
				var e = _.VP(kQ(b.pL));
				if (d && typeof d === "object") {
					if (Array.isArray(d)) return _.Zca(t4b(e), d);
					if (d instanceof e) return d;
					throw Error();
				}
				if (!eQ(a) || a5b(a, b.typeName)) {
					if (d = lQ.get(b.typeName)) return d.Ls(c, a);
				}
				e = new e();
				var f = e.Pd;
				var g = f[_.Ra] | 0;
				var k;
				if (!((k = c[i5b]) == null)) {
					k.forEach((D, G) => {
						_.sc(f, g, G, D, _.hb(g));
					});
				}
				if ((k = _.ec(_.fc)) && (d = c[k])) {
					f[k] = _.nca(d);
				}
				k = b.ayb;
				if (_.ec(mQ) && _.ec(mQ) in a) {
					k = b.oVa;
				}
				if (W4b(a)) {
					k = b.pVa;
				}
				d = new Set();
				var p = new Set();
				for (let [D, G] of Object.entries(_.jb(c, hQ))) {
					var r = D;
					var v = G;
					let L;
					if (_.ec(nQ) && _.ec(nQ) in a && ((L = b.N4) == null ? 0 : L.has(r))) {
						if (!v) continue;
						c = b.oVa.get(v.case);
						v = v.value;
					} else c = k.get(r);
					if (c == null) {
						var w = F4b(b.pL, !!(_.ec(oQ) && _.ec(oQ) in a), W4b(a)).get(r);
						if (w) {
							c = j5b(b.pL, z4b(w));
						}
					}
					if (c == null) {
						c = b.pVa.get(r);
					}
					if (c == null) {
						UP(true);
						continue;
					}
					if (_.ec(k5b) && v === null && c.T5a && !eQ(a)) {
						v = 0;
					}
					r = iQ(a, c, v);
					if (r == null) continue;
					if (v = c.AGa) {
						d.has(v) ? TP() : d.add(v);
					}
					w = c.aDb;
					UP(!p.has(w));
					p.add(w);
					let N = f[_.Ra] | 0;
					if (Array.isArray(r)) for (let Q of r) _.Dc(f, f[_.Ra] | 0, w).push(Q);
					else if (r instanceof Map) for (let Q of r.entries()) _.KPa(f, w, Q);
					else v ? _.Qs(f, w, v, _.jb(r, l5b)) : (c.qFb || !m5b(c, r)) && _.sc(f, N, w, _.jb(r, l5b), _.hb(N));
				}
				_.Ta(f);
				return e;
			}
		};
		o5b = function(a, b, c) {
			if (c != null) {
				var d = b.messageType;
				var e = b.enumType;
				var f = b.HWa;
				if (d) return _.n5b(a, d, _.jb(c, _.r4b(_.APa, q4b(_.h))));
				if (e) {
					b = _.jb(f, _.Js)(c, 0);
					if (b == null) return;
					var g;
					return _.ec(pQ) && _.ec(pQ) in a ? b : (g = _.Q4b(e).get(b)) != null ? g : b;
				}
				e = _.jb(f, _.Js)(c, 0);
				if (_.ec(jQ) && _.ec(jQ) in a && (b.fieldType === 2 || b.fieldType === 1) && typeof e === "string") switch (e) {
					case "NaN": return Number.NaN;
					case "Infinity": return Number.POSITIVE_INFINITY;
					case "-Infinity": return Number.NEGATIVE_INFINITY;
				}
				var k;
				if (g = Y4b(a) && _.Pa()) a: switch (b.fieldType) {
					case 3:
					case 4:
					case 18:
					case 6:
					case 16:
						g = true;
						break a;
					default: g = false;
				}
				if (g && ((k = b.field.getOptions()) == null ? undefined : _.wn(k, 6)) !== 1 && e != null) return BigInt(e);
				if (b.fieldType === 12 && e) {
					if (typeof e === "object" && e instanceof _.bb) return cQ(a) ? _.ep(e) : _.lc(e);
					if (typeof e === "string") return cQ(a) ? _.Vaa(e) : e;
				}
				return e;
			}
		};
		_.n5b = function(a, b, c) {
			var d = c[_.Va] === _.Wa;
			var e = !d && Array.isArray(c);
			if (e) {
				_.xca(c, 500, b.messageId);
			}
			c = d ? c.Pd : e ? c : undefined;
			if (c == null) a = c;
			else a: {
				e = c[_.Ra] | 0;
				if (d = S4b(a) && !!(e & 2)) {
					let f = c[h5b];
					if (f) {
						a = f;
						break a;
					}
				}
				a = p5b(a, b, c, e, d);
				if (d && a != null && typeof a === "object") {
					a[h5b] = c, b = ArrayBuffer.isView(a) ? a : Object.freeze(a), c[h5b] = b;
				}
			}
			return a;
		};
		p5b = function(a, b, c, d, e) {
			if (U4b(a)) {
				var f = lQ.get(b.typeName);
				if (f && f.rLa) return c = _.Zca(t4b(_.VP(kQ(b.pL))), _.ic(c, 0, _.jc)), f.rLa(c, a);
			}
			if (!eQ(a) || a5b(a, b.typeName)) {
				if (f = lQ.get(b.typeName)) return c = _.Zca(t4b(_.VP(kQ(b.pL))), _.ic(c, 0, _.jc)), f.Ks(c, a);
			}
			var g = c5b(a) || e5b(a) ? new Set() : undefined;
			var k = q5b(a, b);
			var p = new Set();
			var r;
			_.dba(c, d, (v, w) => {
				if (!(w == null || _.bba(w) || w && typeof w === "object" && w.UJb === _.tPa && w.size === 0)) {
					var D = b[v];
					if (D == null) {
						var G = D4b(b.pL).get(v);
						if (G) {
							var L = z4b(G);
							D = j5b(b.pL, L);
							L = E4b(G.A(_.WP), L.getName(), !!(_.ec(oQ) && _.ec(oQ) in a), W4b(a));
						}
						if (D == null) {
							UP(true);
							var N;
							((N = r) != null ? N : r = new Map()).set(v, w);
							return;
						}
					} else L = r5b(a, D);
					_.ib(L, _.Js);
					_.ib(D, _.Js);
					N = D.isMap;
					var Q = D.isRepeated;
					G = D.messageType;
					var T;
					if (N && !fQ(a)) {
						if (w instanceof _.dc) {
							w = w.entries();
						} else {
							if (!Array.isArray(w)) {
								w = [], TP();
							}
						}
						var X = G[1];
						var Y = G[2];
						for (let [Fa, Ka] of w) {
							G = Fa;
							w = Ka;
							G = String(G == null ? s5b(a, X) : _.VP(o5b(a, X, G)));
							N = o5b(a, Y, w);
							if (!(_.ec(_.gQ.get) && w === null && D.typeName === "google.protobuf.Value" && !eQ(a) || w != null || N != null)) {
								N = s5b(a, Y);
							}
							var ia = undefined;
							((ia = T) != null ? ia : T = {})[G] = N;
						}
					} else if (Q) {
						if (fQ(a) && N && w instanceof _.dc) {
							w = [...w.entries()];
						}
						T = e5b(a) ? [] : null;
						for (X of _.jb(w, _.zPa)) {
							if (X == null) continue;
							ia = o5b(a, D, X);
							if (ia == null) continue;
							((Y = T) != null ? Y : T = []).push(ia);
						}
					} else if (_.ec(k5b) && D.T5a && w === 0 && !eQ(a)) {
						var fa = true;
						T = null;
					} else T = o5b(a, D, w);
					if (_.ec(_.gQ.get) && T === null && D.typeName === "google.protobuf.Value" && !eQ(a)) {
						fa = true;
					}
					if (e && T != null && typeof T === "object" && !ArrayBuffer.isView(T)) {
						Object.freeze(T);
					}
					if (fa || T != null) D.AGa && _.ec(nQ) && _.ec(nQ) in a && !_.Pm(D.field, 17) ? (fa = _.mj(b.vya, t5b, 8, _.oj())[_.yj(D.field, 9)], fa = bQ(fa.getName()), D = D.zwa, g && g.add(v), p.add(fa), v = T != null && typeof T === "object" && cQ(a) && S4b(a) && T instanceof Uint8Array ? {
						case: D,
						get value() {
							return T.slice();
						}
					} : {
						case: D,
						value: T
					}, k[fa] = e ? Object.freeze(v) : v) : (g && g.add(v), p.has(L) && TP(), p.add(L), T != null && typeof T === "object" && cQ(a) && S4b(a) && T instanceof Uint8Array ? Object.defineProperty(k, L, {
						enumerable: true,
						get() {
							return T.slice();
						}
					}) : k[L] = T);
				}
			});
			if (c5b(a) || e5b(a)) for (let [v, w] of Object.entries(b)) {
				f = v;
				d = w;
				if (Number.isNaN(+f)) continue;
				if (g.has(+f)) continue;
				f = r5b(a, d);
				let D = d.isMap;
				if (d.isRepeated && (!D || fQ(a)) && e5b(a)) {
					k[f] = e ? u5b : [];
				} else {
					if (D && c5b(a) && !fQ(a)) {
						k[f] = e ? v5b : {};
					}
				}
			}
			if (r) {
				k[i5b] = r;
			}
			if ((d = _.ec(_.fc)) && (c = _.hc(c))) {
				k[d] = _.nca(c);
			}
			return k;
		};
		q5b = function(a, b) {
			if (_.ec(w5b) && _.ec(w5b) in a) {
				if (!b.Eaa && (UP(_.ec(nQ)), UP(b.N4), !b.Eaa)) {
					b.Eaa = {};
					b.Daa = {};
					b.W7a = new Set();
					var c = b.vya;
					for (let f of _.mj(c, x5b, 2, _.oj())) {
						c = _.VP(b[_.yj(f, 3)]);
						let g = c.AGa;
						let k = c.isRepeated;
						let p = c.isMap;
						let r = c.fieldType;
						let v = c.fieldPresence;
						let w = r5b(a, c);
						if (k || p) b.Daa[w] = p ? v5b : u5b;
						else if (!g && r !== 11 && r !== 10 && !_.Pm(f, 17)) switch (b.Eaa[w] = c.V7a, v) {
							case 1:
							case 3:
								b.W7a.add(w);
								break;
							case 2:
								b.Daa[w] = c.V7a;
								break;
							default: TP(`unknown field presence ${v} for field ${f.getName()} in ${b.typeName}`);
						}
					}
					var d;
					var e;
					for (let f of (e = (d = b.N4) == null ? undefined : d.keys()) != null ? e : []) b.Daa[f] = y5b;
					a = b.pL.fieldPresence;
					if (a === 1 || a === 3 || b.W7a.size > 0) b.CUb = true;
				}
				return b.CUb ? (a = Object.create(b.Eaa, { $typeName: {
					value: b.typeName,
					enumerable: true
				} }), Object.assign(a, b.Daa), a) : Object.assign({ $typeName: b.typeName }, b.Eaa, b.Daa);
			}
			return {};
		};
		_.B5b = function(a) {
			if (a != null) {
				var b = a[z5b];
				if (b != null) return b;
				if ((b = a[$P]) == null) {
					XP(), b = a[$P] = A5b(a.A);
				}
				var c;
				var d = a[z5b] = {
					typeName: a.getTypeName(),
					pL: a,
					vya: b,
					messageId: (c = kQ(a)) == null ? undefined : c.messageId
				};
				if (!!kQ(a)) {
					new (kQ(a))();
				}
				if (_.ec(nQ)) {
					d.N4 = new Map();
				}
				c = d.ayb = new Map();
				var e = _.ec(mQ) ? d.oVa = new Map() : undefined;
				var f = d.pVa = new Map();
				var g = new Map();
				for (let p of _.mj(b, x5b, 2, _.oj())) {
					let r;
					if (_.dn(p, 9)) {
						var k = _.yj(p, 9);
						r = g.get(k);
						if (r == null) {
							g.set(k, r = []);
						}
						r.push(_.yj(p, 3));
						if (d.N4 && !_.Pm(p, 17)) {
							let v = _.mj(b, t5b, 8, _.oj())[k];
							d.N4.set(bQ(v.getName()), k);
						}
					}
					k = j5b(a, p, r);
					if (k != null) {
						d[_.yj(p, 3)] = k, c.set(k.jsonName, k), f.set(p.getName(), k), e && e.set(k.zwa, k);
					}
				}
				return d;
			}
		};
		r5b = function(a, b) {
			return W4b(a) ? b.field.getName() : _.ec(mQ) && _.ec(mQ) in a ? b.zwa : b.jsonName;
		};
		s5b = function(a, b) {
			switch (b.fieldType) {
				case 11:
				case 10: return {};
				default: return _.ec(pQ) && _.ec(pQ) in a && b.enumType ? b.fYa : b.defaultValue;
			}
		};
		j5b = function(a, b, c) {
			var d = b[C5b];
			if (d) return d;
			d = b.getType();
			var e = b.Bl() === 3;
			var f = d === 11 || d === 10;
			var g = f ? _.B5b(w4b(b.getTypeName())) : undefined;
			if (g) {
				var k;
				var p = (k = g.vya.getOptions()) == null ? undefined : _.Pm(k, 7);
				if (!e) {
					e = !!p;
				}
			} else f && TP(`unknown message type for field ${b.getName()} in ${a.getTypeName()}`);
			k = _.Ss(b, D5b, 8);
			k = _.Ss(k, E5b, 21);
			k = _.wn(k, 1);
			k = k != null ? k : a.fieldPresence;
			f = !e && !p && (k !== 2 || !!c);
			if (d === 14) {
				var r = b.getTypeName();
				if (r.indexOf(".") === 0) {
					r = r.substring(1);
				}
				r = _.KQa.get(r);
				r = _.jb(r, _.Js);
			} else r = undefined;
			var v = _.Xb(_.tn(b, 6));
			v = (v == null ? 0 : v.startsWith(".")) ? v.substring(1) : v;
			var w = bQ(b.getName());
			var D;
			if (_.ec(k5b) && r && c && v === "google.protobuf.NullValue") {
				D = true;
			}
			var G;
			return b[C5b] = {
				parent: a,
				field: b,
				fieldType: d,
				aDb: _.yj(b, 3),
				isRepeated: e,
				qFb: f,
				fieldPresence: k,
				typeName: v,
				isMap: p,
				AGa: c,
				messageType: g,
				enumType: r,
				zwa: w,
				jsonName: (G = _.Xb(_.tn(b, 10))) != null ? G : w,
				HWa: F5b(d),
				defaultValue: G5b(b, r),
				fYa: G5b(b, r, true),
				V7a: G5b(b, r, true, true),
				T5a: D
			};
		};
		I5b = function(a, b) {
			if (a == null) return a;
			if (b === 1 && typeof a === "number") {
				UP(Math.abs(a) <= 34028234663852886e22);
			}
			return H5b(a, b);
		};
		H5b = function(a, b) {
			if (a == null) return a;
			if (b === 1) switch (typeof a) {
				case "number":
					UP(Number.isFinite(a));
					break;
				case "string": UP(a.length && (!isNaN(a) || a === "NaN")), a = Number(a);
			}
			a = _.ub(a);
			return b !== 0 || Number.isFinite(a) ? a : String(a);
		};
		J5b = function(a) {
			if (a == null) return a;
			a = _.Xb(a);
			if (a != null) {
				_.iaa(a);
			}
			return a;
		};
		K5b = function(a) {
			if (a == null) return a;
			if (typeof a === "string") {
				a = _.Uaa(a), UP(atob(btoa(a)) === a);
			}
			if (typeof a === "object" && a instanceof Uint8Array) {
				a = _.cb(new Uint8Array(a));
			} else {
				a = a == null || a instanceof _.bb ? a : typeof a === "string" ? _.ab(a) : undefined;
			}
			return a;
		};
		L5b = function(a, b) {
			if (b === 1) {
				UP(a == null || typeof a === "boolean");
			}
			return _.wb(a);
		};
		N5b = function(a, b) {
			if (a == null) return a;
			var c = Number(a);
			if (b === 1) {
				typeof a === "string" && UP(M5b.test(a)), UP(Number.isSafeInteger(c) && c >= 0 && c <= 4294967295);
			}
			return _.Fb(a);
		};
		O5b = function(a, b) {
			if (a == null) return a;
			var c = Number(a);
			if (b === 1) {
				typeof a === "string" && UP(M5b.test(a)), UP(Number.isSafeInteger(c) && c >= -2147483648 && c <= 2147483647);
			}
			return _.Db(a);
		};
		P5b = function(a, b) {
			if (a == null) return a;
			if (b === 1 && _.Pa()) {
				b = BigInt(a), UP(b >= BigInt(0)), UP(b <= BigInt("18446744073709551615"));
			}
			return s4b(a, true);
		};
		Q5b = function(a, b) {
			if (a == null) return a;
			if (b === 1 && _.Pa()) {
				b = BigInt(a), UP(b >= BigInt("-9223372036854775808")), UP(b <= BigInt("9223372036854775807"));
			}
			return _.Dba(a, true);
		};
		F5b = function(a) {
			switch (a) {
				case 1: return H5b;
				case 2: return I5b;
				case 8: return L5b;
				case 9: return J5b;
				case 12: return K5b;
				case 7:
				case 13: return N5b;
				case 5:
				case 14:
				case 15:
				case 17: return O5b;
				case 4:
				case 6: return P5b;
				case 3:
				case 18:
				case 16: return Q5b;
			}
		};
		G5b = function(a, b, c = false, d = false) {
			var e = _.Xb(_.tn(a, 7));
			switch (a.getType()) {
				case 1:
				case 2:
					switch (e == null ? undefined : e.toLowerCase()) {
						case "inf": return Infinity;
						case "-inf": return -Infinity;
						case "nan": return NaN;
					}
					return Number(e != null ? e : 0);
				case 7:
				case 13:
				case 5:
				case 15:
				case 17: return Number(e != null ? e : 0);
				case 8: return e === "true";
				case 9: return e != null ? e : "";
				case 12: return d ? e ? new TextEncoder().encode(e) : new Uint8Array(0) : e != null ? e : "";
				case 14: return c ? e ? N4b(R5b(aQ(b)).find((g) => g.getName() === e)) : N4b(R5b(aQ(b))[0]) : e != null ? e : R5b(aQ(b))[0].getName();
				case 4:
				case 6:
				case 3:
				case 18:
				case 16:
					let f;
					return d && _.Pa() && ((f = a.getOptions()) == null ? undefined : _.wn(f, 6)) !== 1 ? BigInt(e != null ? e : "0") : e != null ? e : "0";
			}
		};
		S5b = function(a, b) {
			if (!Number.isSafeInteger(a) || !Number.isSafeInteger(b) || Math.abs(a) > 315576e6 || Math.abs(b) > 999999999) {
				TP();
			}
			return true;
		};
		T5b = function(a, b) {
			if (!Number.isSafeInteger(a) || !Number.isSafeInteger(b) || a > 253402300799 || a < -62135596800 || b > 999999999 || b < 0) {
				TP();
			}
			return true;
		};
		m5b = function(a, b) {
			if (b instanceof _.bb) {
				b = _.lc(b);
			}
			a = a.fYa;
			return a != null && a === b;
		};
		_.Cy.prototype.sja = _.ca(112, function() {
			return _.uj(this, 1, _.oj());
		});
		_.mpb.prototype.sja = _.ca(111, function() {
			return _.uj(this, 1, _.oj());
		});
		_.no.prototype.ao = _.ca(7, function() {
			var a = {};
			_.zc(this, 1, _.qo).forEach(function(b, c) {
				a[c] = b.ao();
			});
			return a;
		});
		_.qo.prototype.ao = _.ca(6, function() {
			switch (_.jj(this, _.yo)) {
				case 1: return null;
				case 2: return _.zo(this, 2, _.yo);
				case 3: return _.qj(this, 3, _.yo);
				case 4: return _.Ao(this, 4, _.yo);
				case 5: return _.fj(this, _.no, 5, _.yo).ao();
				case 6: return _.fj(this, _.uo, 6, _.yo).ao();
				default: throw Error("Ha");
			}
		});
		_.uo.prototype.ao = _.ca(5, function() {
			var a = [];
			var b = _.qBa(this);
			for (let c = 0; c < b.length; c++) a[c] = b[c].ao();
			return a;
		});
		_.WP = {};
		U5b = class {
			constructor() {
				if (_.WP !== _.WP) throw Error();
			}
		};
		V5b = class {
			constructor() {
				if (_.WP !== _.WP) throw Error();
			}
		};
		D4b = function(a) {
			XP();
			var b = _.LQa.get(a.typeName);
			if (b == null) {
				_.LQa.set(a.typeName, b = new Map());
			}
			return b;
		};
		kQ = function(a) {
			XP();
			return a.ctor;
		};
		u4b = class extends U5b {
			constructor(a, b, c, d, e) {
				super();
				this.ctor = a;
				this.typeName = b;
				this.fieldPresence = c;
				this.A = d;
				_.WOa(e);
				_.JQa.set(b, this);
				if (a) {
					this.ctor[_.HQa] = d;
				}
			}
			getTypeName() {
				return this.typeName;
			}
		};
		v4b = class extends V5b {
			constructor(a, b) {
				super();
				this.typeName = a;
				this.A = b;
				XP();
				_.KQa.set(a, this);
			}
			getTypeName() {
				return this.typeName;
			}
		};
		_.X5b = function(a) {
			var b = new _.no();
			var c = _.zc(b, 1, _.qo);
			for (let d in a) {
				let e = a[d];
				if (e !== undefined) {
					c.set(d, W5b(e));
				}
			}
			return b;
		};
		W5b = function(a) {
			var b = new _.qo();
			switch (_.vb(a)) {
				case "string": return _.to(b, a);
				case "number": return _.so(b, a);
				case "boolean": return _.nBa(b, a);
				case "null": return _.ro(b, 0);
				case "array": return _.vo(b, Y5b(a));
				case "object": return _.wo(b, _.X5b(a));
				default: throw Error("Ga");
			}
		};
		Y5b = function(a) {
			var b = new _.uo();
			for (let c = 0; c < a.length; c++) _.oBa(b, W5b(a[c]));
			return b;
		};
		Z5b = _.Wc(_.$u);
		var E5b = class extends _.h {
			constructor(a) {
				super(a, 10);
			}
		};
		var $5b = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
		};
		var a6b = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
		};
		var b6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getName() {
				return _.l(this, 1);
			}
			getOptions() {
				return _.Z(this, a6b, 3);
			}
			setOptions(a) {
				return _.ln(this, a6b, 3, a);
			}
			F() {
				return _.sn(this, a6b, 3);
			}
		};
		var D5b = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
		};
		var x5b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getName() {
				return _.l(this, 1);
			}
			Bl() {
				return _.Lm(this, 4, 1);
			}
			hasLabel() {
				return _.wn(this, 4) != null;
			}
			getType() {
				return _.Lm(this, 5, 1);
			}
			getTypeName() {
				return _.l(this, 6);
			}
			getOptions() {
				return _.Z(this, D5b, 8);
			}
			setOptions(a) {
				return _.ln(this, D5b, 8, a);
			}
			F() {
				return _.sn(this, D5b, 8);
			}
		};
		var c6b = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
		};
		var d6b = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
		};
		var t5b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getName() {
				return _.l(this, 1);
			}
			getOptions() {
				return _.Z(this, d6b, 2);
			}
			setOptions(a) {
				return _.ln(this, d6b, 2, a);
			}
			F() {
				return _.sn(this, d6b, 2);
			}
		};
		var A5b = _.$c(class extends _.h {
			constructor(a) {
				super(a);
			}
			getName() {
				return _.l(this, 1);
			}
			getOptions() {
				return _.Z(this, c6b, 7);
			}
			setOptions(a) {
				return _.ln(this, c6b, 7, a);
			}
			F() {
				return _.sn(this, c6b, 7);
			}
		});
		var x4b = _.$c(class extends _.h {
			constructor(a) {
				super(a);
			}
			getName() {
				return _.l(this, 1);
			}
			setValue(a, b) {
				return _.Os(this, 2, b6b, a, b);
			}
			getOptions() {
				return _.Z(this, $5b, 3);
			}
			setOptions(a) {
				return _.ln(this, $5b, 3, a);
			}
			F() {
				return _.sn(this, $5b, 3);
			}
		});
		var y4b = _.$c(x5b);
		var $P = Symbol();
		var C4b = Symbol();
		var B4b = Symbol();
		var A4b = Symbol();
		var I4b = RegExp("_[a-z]?", "g");
		var J4b = RegExp("[A-Z]", "g");
		var G4b = String.fromCharCode;
		var L4b = Symbol();
		var P4b = Symbol();
		_.e6b = {};
		_.e6b.get = _.YP(_.$u, "google.protobuf.Any", 2, "[null,[[\"type_url\",null,1,1,9,null,null,[2]],[\"value\",null,2,1,12,null,null,[1]]]]");
		_.f6b = {};
		_.f6b.get = _.YP(_.kp, "google.protobuf.Duration", 2, "[null,[[\"seconds\",null,1,1,3],[\"nanos\",null,2,1,5]]]");
		var g6b = {};
		g6b.get = _.YP(_.Cy, "google.protobuf.FieldMask", 2, "[null,[[\"paths\",null,1,3,9]]]");
		var k5b = _.ZP("google.protobuf.NullValue", "[null,[[\"NULL_VALUE\",0]]]");
		var h6b;
		h6b = {};
		_.qQ = {};
		i6b = {};
		_.gQ = {};
		i6b.get = _.YP(undefined, "google.protobuf.Struct.FieldsEntry", 2, "[null,[[\"key\",null,1,1,9],[\"value\",null,2,1,11,\".google.protobuf.Value\"]],null,null,null,null,[null,null,null,null,null,null,1]]", _.gQ.get);
		_.qQ.get = _.YP(_.no, "google.protobuf.Struct", 2, "[null,[[\"fields\",null,1,3,11,\".google.protobuf.Struct.FieldsEntry\"]]]", i6b.get);
		_.gQ.get = _.YP(_.qo, "google.protobuf.Value", 2, "[null,[[\"null_value\",null,1,1,14,\".google.protobuf.NullValue\",null,null,0],[\"number_value\",null,2,1,1,null,null,null,0],[\"string_value\",null,3,1,9,null,null,null,0],[\"bool_value\",null,4,1,8,null,null,null,0],[\"struct_value\",null,5,1,11,\".google.protobuf.Struct\",null,null,0],[\"list_value\",null,6,1,11,\".google.protobuf.ListValue\",null,null,0]],null,null,null,null,null,[[\"kind\"]]]", h6b.get, _.qQ.get, k5b);
		h6b.get = _.YP(_.uo, "google.protobuf.ListValue", 2, "[null,[[\"values\",null,1,3,11,\".google.protobuf.Value\"]]]", _.gQ.get);
		_.j6b = {};
		_.j6b.get = _.YP(_.Zo, "google.protobuf.Timestamp", 2, "[null,[[\"seconds\",null,1,1,3],[\"nanos\",null,2,1,5]]]");
		var k6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Pm(this, 1);
			}
			setValue(a) {
				return _.cq(this, 1, a);
			}
		};
		var l6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.fp(this, 1);
			}
			setValue(a) {
				return _.nt(this, 1, a);
			}
		};
		var m6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Vm(this, 1);
			}
			setValue(a) {
				return _.lt(this, 1, a);
			}
		};
		var n6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Vm(this, 1);
			}
			setValue(a) {
				return _.lt(this, 1, a);
			}
		};
		var o6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.yj(this, 1);
			}
			setValue(a) {
				return _.gt(this, 1, a);
			}
		};
		var p6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Ys(this, 1);
			}
			HAa() {
				return _.$s(this, 1);
			}
			setValue(a) {
				return _.jt(this, 1, a);
			}
		};
		var q6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.l(this, 1);
			}
			setValue(a) {
				return _.Uc(this, 1, a);
			}
		};
		var r6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Xs(this, 1);
			}
			setValue(a) {
				return _.Bc(this, 1, _.Eb(a), 0);
			}
		};
		var s6b = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Zs(this, 1);
			}
			HAa() {
				var a = s4b(_.tn(this, 1, undefined, undefined, _.Vb));
				return a != null ? a : "0";
			}
			setValue(a) {
				return _.Bc(this, 1, a == null ? a : _.Eba(a), "0");
			}
		};
		var t6b = {};
		t6b.get = _.YP(k6b, "google.protobuf.BoolValue", 2, "[null,[[\"value\",null,1,1,8]]]");
		var u6b = {};
		u6b.get = _.YP(l6b, "google.protobuf.BytesValue", 2, "[null,[[\"value\",null,1,1,12,null,null,[1]]]]");
		var v6b = {};
		v6b.get = _.YP(m6b, "google.protobuf.DoubleValue", 2, "[null,[[\"value\",null,1,1,1]]]");
		var w6b = {};
		w6b.get = _.YP(n6b, "google.protobuf.FloatValue", 2, "[null,[[\"value\",null,1,1,2]]]");
		var x6b = {};
		x6b.get = _.YP(o6b, "google.protobuf.Int32Value", 2, "[null,[[\"value\",null,1,1,5]]]");
		var y6b = {};
		y6b.get = _.YP(p6b, "google.protobuf.Int64Value", 2, "[null,[[\"value\",null,1,1,3]]]");
		var z6b = {};
		z6b.get = _.YP(q6b, "google.protobuf.StringValue", 2, "[null,[[\"value\",null,1,1,9]]]");
		var A6b = {};
		A6b.get = _.YP(r6b, "google.protobuf.UInt32Value", 2, "[null,[[\"value\",null,1,1,13]]]");
		var B6b = {};
		B6b.get = _.YP(s6b, "google.protobuf.UInt64Value", 2, "[null,[[\"value\",null,1,1,4]]]");
		var V4b = Symbol();
		var pQ = Symbol();
		var X4b = Symbol();
		var dQ = Symbol();
		var b5b = Symbol();
		var d5b = Symbol();
		var oQ = Symbol();
		var mQ = Symbol();
		var f5b = Symbol();
		var R4b = Symbol();
		var nQ = Symbol();
		var Z4b = Symbol();
		var w5b = Symbol();
		var T4b = Symbol();
		var jQ = Symbol();
		var i5b = Symbol();
		var u5b = Object.freeze([]);
		var v5b = Object.freeze({});
		var y5b = Object.freeze({ case: undefined });
		var h5b = Symbol();
		Object.freeze([]);
		Object.freeze([oQ]);
		Object.freeze([pQ, oQ]);
		Object.freeze([
			oQ,
			f5b,
			mQ,
			dQ
		]);
		Object.freeze([
			pQ,
			oQ,
			mQ,
			dQ
		]);
		Object.freeze([
			nQ,
			w5b,
			T4b,
			X4b,
			pQ,
			Z4b,
			jQ,
			mQ,
			R4b,
			b5b,
			d5b,
			dQ
		]);
		Object.freeze([
			pQ,
			dQ,
			V4b
		]);
		var $4b = new Set([
			"google.protobuf.Timestamp",
			"google.protobuf.Duration",
			"google.protobuf.FieldMask",
			"google.protobuf.Value",
			"google.protobuf.ListValue"
		]);
		var z5b = Symbol();
		var C5b = Symbol();
		var M5b = RegExp("^-?[0-9]+([Ee][+-]?[0-9]+)?$");
		var l5b = _.kb((a) => {
			if (a == null) return false;
			switch (typeof a) {
				case "number":
				case "boolean":
				case "string":
				case "bigint": return true;
				default: return a instanceof _.bb || a instanceof _.h;
			}
		});
		var hQ = _.kb((a) => a != null && typeof a === "object" && !Array.isArray(a) && a.constructor === Object);
		var lQ = new Map();
		if (_.ec(_.e6b.get)) {
			lQ.set("google.protobuf.Any", {
				Ks(a, b) {
					a = _.jb(a, _.Zn(_.$u));
					if (U4b(b)) {
						b = _.tn(a, 2);
						if (b instanceof Uint8Array) b = new Uint8Array(b);
						else if (b instanceof _.bb) b = _.ep(b);
						else if (typeof b === "string") b = _.ep(_.ab(b));
						else if (b == null) b = new Uint8Array(0);
						else throw Error("Pf");
						return {
							$typeName: "google.protobuf.Any",
							typeUrl: _.l(a, 1),
							value: b
						};
					}
					if (_.l(a, 1) === "" && _.tn(a, 2) == null) return {};
					var c = a.getTypeName();
					var d = w4b(c);
					if (d == null) {
						TP();
					}
					var e = _.B5b(d);
					if (e == null) {
						TP();
					}
					var f = _.tn(a, 2);
					if (typeof f === "string" || f instanceof _.bb || f instanceof Uint8Array) {
						TP();
					}
					d = _.VP(kQ(d));
					if (a.bla()) {
						d = t4b(d), d = _.Vc(a, d, c);
					} else {
						d = _.Vc(a, d, c);
					}
					b = _.n5b(b, e, d);
					if (b == null) {
						TP();
					}
					if (lQ.has(c)) return {
						"@type": _.l(a, 1),
						value: b
					};
					b = _.jb(b, hQ);
					b["@type"] = _.l(a, 1);
					return b;
				},
				Ls(a, b) {
					a = _.jb(a, hQ);
					if (U4b(b)) {
						b = a.typeUrl;
						var c = a.value;
						c = c instanceof Uint8Array ? new Uint8Array(c) : new Uint8Array(0);
						if (!b && c.length === 0) return Z5b();
						b = b ? b.substring(b.lastIndexOf("/") + 1) : "";
						return _.rp(new _.$u().pack(c, b));
					}
					var { "@type": d } = a;
					if (d == null) {
						for (c in a) c !== "@type" && TP();
						return Z5b();
					}
					_.ib(d, _.mb);
					if (!(d.indexOf("type.googleapis.com/") === 0)) {
						TP();
					}
					c = d.substring(20);
					d = w4b(c);
					if (d == null) {
						TP();
					}
					d = _.B5b(d);
					if (d == null) {
						TP();
					}
					var e = lQ.has(c);
					a = e ? a.value : Object.assign({}, a);
					if (!e) {
						delete _.jb(a, hQ)["@type"];
					}
					b = g5b(b, d, a);
					if (b == null) {
						TP();
					}
					a = new _.$u();
					d = b.constructor;
					b.oPb = undefined;
					_.Zda(a, c, "type.googleapis.com");
					_.ln(a, d, 2, b, undefined);
					return _.oc(a);
				}
			});
		}
		if (_.ec(t6b.get)) {
			lQ.set("google.protobuf.BoolValue", {
				Ks(a) {
					return _.jb(a, _.Zn(k6b)).getValue();
				},
				Ls(a) {
					return _.rp(new k6b().setValue(_.jb(a, _.iba)));
				}
			});
		}
		if (_.ec(u6b.get)) {
			lQ.set("google.protobuf.BytesValue", {
				Ks(a, b) {
					a = _.jb(a, _.Zn(l6b)).getValue();
					return cQ(b) ? _.ep(a) : _.lc(a);
				},
				Ls(a) {
					a = _.jb(a, _.r4b(_.mb, q4b(Uint8Array)));
					return _.rp(new l6b().setValue(K5b(a, 1)));
				}
			});
		}
		if (_.ec(w6b.get)) {
			lQ.set("google.protobuf.FloatValue", {
				Ks(a) {
					return _.jb(a, _.Zn(n6b)).getValue();
				},
				Ls(a) {
					return _.rp(new n6b().setValue(I5b(a, 1)));
				}
			});
		}
		if (_.ec(v6b.get)) {
			lQ.set("google.protobuf.DoubleValue", {
				Ks(a) {
					return _.jb(a, _.Zn(m6b)).getValue();
				},
				Ls(a) {
					return _.rp(new m6b().setValue(H5b(a, 1)));
				}
			});
		}
		if (_.ec(x6b.get)) {
			lQ.set("google.protobuf.Int32Value", {
				Ks(a) {
					return _.jb(a, _.Zn(o6b)).getValue();
				},
				Ls(a) {
					return _.rp(new o6b().setValue(O5b(a, 1)));
				}
			});
		}
		if (_.ec(y6b.get)) {
			lQ.set("google.protobuf.Int64Value", {
				Ks(a) {
					return _.jb(a, _.Zn(p6b)).HAa();
				},
				Ls(a) {
					return _.rp(new p6b().setValue(Q5b(a, 1)));
				}
			});
		}
		if (_.ec(A6b.get)) {
			lQ.set("google.protobuf.UInt32Value", {
				Ks(a) {
					return _.jb(a, _.Zn(r6b)).getValue();
				},
				Ls(a) {
					return _.rp(new r6b().setValue(N5b(a, 1)));
				}
			});
		}
		if (_.ec(B6b.get)) {
			lQ.set("google.protobuf.UInt64Value", {
				Ks(a) {
					return _.jb(a, _.Zn(s6b)).HAa();
				},
				Ls(a) {
					return _.rp(new s6b().setValue(P5b(a, 1)));
				}
			});
		}
		if (_.ec(z6b.get)) {
			lQ.set("google.protobuf.StringValue", {
				Ks(a) {
					return _.jb(a, _.Zn(q6b)).getValue();
				},
				Ls(a) {
					return _.rp(new q6b().setValue(J5b(a, 1)));
				}
			});
		}
		var C6b = RegExp("(?:|0{3}|0{6}|0{9})$");
		if (_.ec(_.f6b.get)) {
			lQ.set("google.protobuf.Duration", {
				Ks(a) {
					var b = _.jb(a, _.Zn(_.kp));
					var c = b.getSeconds();
					a = b.Cl();
					var d = Number(c);
					if (S5b(d, a)) return c = (Math.abs(a) / 1e9).toFixed(9).substring(2), c = c.replace(C6b, ""), a = (d < 0 || a < 0 ? "-" : "") + `${Math.abs(d)}`, c.length > 0 && (a += `.${c}`), a + "s";
				},
				rLa(a, b) {
					var c = _.jb(a, _.Zn(_.kp));
					var d = c.getSeconds();
					a = c.Cl();
					if (Y4b(b) && _.Pa()) {
						d = BigInt(d);
					}
					return {
						$typeName: "google.protobuf.Duration",
						seconds: d,
						nanos: a
					};
				},
				Ls(a) {
					var b;
					if (b = typeof a === "string") {
						b = a;
						b = b.lastIndexOf("s") === Math.max(0, b.length - 1);
					}
					if (!b) {
						TP();
					}
					a = a.substring(0, a.length - 1);
					b = _.jb(a, _.mb).split(".");
					if (b.length > 2) {
						TP();
					}
					var c = a[0] === "-";
					a = Number(b[0]);
					b = b[1];
					b = b == null ? 0 : Number(b + "0".repeat(9 - b.length)) * (c ? -1 : 1);
					if (S5b(a, b)) return a = new _.kp().setSeconds(a), a = _.gt(a, 2, b), _.oc(a);
				}
			});
		}
		var D6b = RegExp("((0){3})+$", "gi");
		var E6b = RegExp("\\.[0-9]{3}Z", "gi");
		var F6b = RegExp("^(\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2})(\\.\\d{1,9})?([+-]\\d{2}:\\d{2}|Z)$");
		if (_.ec(_.j6b.get)) {
			lQ.set("google.protobuf.Timestamp", {
				Ks(a) {
					var b = _.jb(a, _.Zn(_.Zo));
					var c = b.getSeconds();
					a = b.Cl();
					c = Number(c);
					if (T5b(c, a)) {
						var d = String(a).padStart(9, "0").replace(D6b, "");
						return new Date(c * 1e3 + Math.trunc(a / 1e9)).toISOString().replace(E6b, (d.length > 0 ? `.${d}` : "") + "Z");
					}
				},
				rLa(a, b) {
					var c = _.jb(a, _.Zn(_.Zo));
					var d = c.getSeconds();
					a = c.Cl();
					if (Y4b(b) && _.Pa()) {
						d = BigInt(d);
					}
					return {
						$typeName: "google.protobuf.Timestamp",
						seconds: d,
						nanos: a
					};
				},
				Ls(a) {
					if (!(typeof a === "string" && F6b.test(a))) {
						TP();
					}
					var b = new Date(a).getTime() / 1e3;
					a = a.match(F6b);
					b = Math.floor(b);
					a = Math.trunc(a[2] != null ? Number(a[2]) * 1e9 : 0);
					if (T5b(b, a)) return _.rp(_.fq(new _.Zo().setSeconds(b), a));
				}
			});
		}
		if (_.ec(g6b.get)) {
			lQ.set("google.protobuf.FieldMask", {
				Ks(a) {
					return _.jb(a, _.Zn(_.Cy)).sja().map((b) => {
						var c = b.split(".").map(bQ).join(".");
						var d = c.split(".").map(K4b).join(".");
						UP(b === d);
						return c;
					}).join(",");
				},
				Ls(a) {
					return _.rp(_.bbb(new _.Cy(), _.jb(a, _.mb).split(",").filter((b) => {
						UP(!b.includes("_"));
						return b.length > 0;
					}).map((b) => b.split(".").map(K4b).join("."))));
				}
			});
		}
		if (_.ec(_.qQ.get)) {
			lQ.set("google.protobuf.Struct", {
				Ks(a) {
					return _.jb(a, _.Zn(_.no)).ao();
				},
				Ls(a) {
					return _.rp(_.X5b(_.jb(a, hQ)));
				}
			});
		}
		if (_.ec(_.gQ.get)) {
			lQ.set("google.protobuf.Value", {
				Ks(a) {
					a = _.jb(a, _.Zn(_.qo)).ao();
					if (!(typeof a !== "number" || Number.isFinite(a))) {
						TP();
					}
					return a;
				},
				Ls(a) {
					return _.rp(W5b(a));
				}
			});
		}
		if (_.ec(h6b.get)) {
			lQ.set("google.protobuf.ListValue", {
				Ks(a) {
					return _.jb(a, _.Zn(_.uo)).ao();
				},
				Ls(a) {
					return _.rp(Y5b(_.jb(a, _.APa)));
				}
			});
		}
		var rQ;
		var R6b;
		var T6b;
		var V6b;
		var W6b;
		var tQ;
		var X6b;
		var Z6b;
		var $6b;
		var c7b;
		var d7b;
		var t7b;
		var s7b;
		var u7b;
		var v7b;
		G6b = function(a) {
			return _.Gf((b) => b.id === a);
		};
		_.H6b = function(a, ...b) {
			var c = [a[0]];
			for (let d = 0; d < b.length; d++) c.push(String(b[d])), c.push(a[d + 1]);
			return _.fd(c.join(""));
		};
		I6b = function(a) {
			if (a & 1) {
				_.Dh(0, "div", 2), _.Yh(1, 3), _.Eh();
			}
		};
		J6b = function() {
			if (!_.gP.p2) {
				var a = {};
				if (typeof window !== "undefined") {
					a = clientFlags || {};
				}
				_.gP.p2 = typeof a === "string" ? JSON.parse(a) : a;
			}
			a = _.gP.p2;
			if (!_.gP.bHa) {
				var b = _.h1b();
				_.gP.bHa = b.pantheon_flags_init_args || {};
			}
			b = _.gP.bHa;
			return (a = a.console_doc_base_url !== undefined ? a.console_doc_base_url : b.console_doc_base_url) ? _.kd(a) : undefined;
		};
		K6b = function() {
			var a = J6b();
			return (_.u1b() || _.cP(_.$3b)) && a ? a : (0, _.H6b)`https://cloud.google.com/`;
		};
		L6b = function() {
			return J6b() || (0, _.H6b)`https://docs.cloud.google.com/`;
		};
		rQ = function(a) {
			return `AnalyticsService_${a[0]}`;
		};
		N6b = function() {
			return Object.values(M6b).map((a) => a.window);
		};
		O6b = function(a) {
			var b;
			return ((b = M6b[a]) == null ? undefined : b.window) || null;
		};
		Q6b = function(a, b) {
			if (b) {
				if (typeof b === "number") return { visualElementId: b };
				a = b;
			} else {
				a: {
					b = Object.keys(P6b);
					for (let c of b) if (b = a, b.classList.contains(c) || b.hasAttribute(c) || b.tagName.toLowerCase() === c) {
						a = P6b[c];
						break a;
					}
					a = undefined;
				}
				if (a) return { visualElementId: a };
				a = "";
			}
			return { visualElementName: a };
		};
		R6b = function(a, b) {
			if (_.aP()) sharedHostData.trackElement(a, b, undefined);
			else if (a.addEventListener) {
				var c = new _.Jf(a);
				(sQ.instance ? sQ.instance : new sQ()).trackElement(c, Object.assign(Object.assign({}, { trackClick: true }, undefined), Q6b(a, b)));
				a.addEventListener("click", () => {
					(sQ.instance ? sQ.instance : new sQ()).logEvent("click", a);
				});
			}
		};
		_.S6b = function(a) {
			a = a.querySelectorAll(".mat-focus-indicator");
			for (let b of a) b.classList.add("cm-mat-focus-indicator");
		};
		T6b = function(a) {
			if (a && !a.id) {
				a.id = `cfc-labelledby-message-${_.iu()}`;
			}
		};
		V6b = function(a, b) {
			if (a) {
				var c = tQ(a);
				if (!c.some((d) => d.trim() === b.trim())) {
					c.length === 0 && (T6b(a), c.push(a.id.trim())), c.push(b.trim()), U6b(a, "aria-labelledby", c.join(" "));
				}
			}
		};
		W6b = function(a, b) {
			if (a) {
				var c = tQ(a).filter((d) => d !== b.trim());
				if (c.length === 1 && c[0] === a.id) {
					a.removeAttribute("aria-labelledby");
				} else {
					U6b(a, "aria-labelledby", c.join(" "));
				}
			}
		};
		tQ = function(a) {
			return a ? (a.getAttribute("aria-labelledby") || "").match(/\S+/g) || [] : [];
		};
		X6b = function(a, b) {
			return typeof a === "string" ? `${b || ""}/${a}` : a;
		};
		Y6b = function(a, b) {
			a = new _.hk(a);
			if (b) {
				_.On(a, "authuser", b);
			}
			return a.toString();
		};
		Z6b = function(a) {
			if (a & 1) {
				_.R(0, "\xA0");
			}
		};
		$6b = function(a) {
			if (a & 1) {
				_.R(0, " ");
			}
		};
		b7b = function(a) {
			if (a.type === 1 || a.type === 3) {
				if (!a.path) throw Error("ag");
				if (!a.path.startsWith("/")) throw Error("bg");
				a = _.hd((a.type === 1 ? K6b : L6b)()).slice(0, -1) + a.path;
			} else if (a.type === 2) {
				if (!a.suffix) throw Error("cg");
				a = "https://support.google.com/" + a7b.supportUrlPrefix + a.suffix;
			} else if (a.type === 999) {
				if (!a.url) throw Error("dg");
				a = a.url;
			} else throw Error("eg");
			return a;
		};
		c7b = function(a) {
			if (a & 1) {
				_.R(0, "\xA0");
			}
		};
		d7b = function(a) {
			if (a & 1) {
				_.R(0, " ");
			}
		};
		f7b = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 4);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(e7b(c));
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.wh("aria-label", a.ariaLabel), _.y(), _.S(" ", a.label, " ");
			}
		};
		g7b = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 4);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(e7b(c));
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.wh("href", a.href, _.rg)("target", a.target || null)("aria-label", a.ariaLabel), _.y(), _.S(" ", a.label, " ");
			}
		};
		h7b = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 5);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(e7b(c));
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("cfcDocLink", a.A6b), _.wh("aria-label", a.ariaLabel), _.y(), _.S(" ", a.label, " ");
			}
		};
		i7b = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 6);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(e7b(c));
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				a = _.K(), _.E("cfcDocHref", a.z6b), _.wh("aria-label", a.ariaLabel), _.y(), _.S(" ", a.label, " ");
			}
		};
		j7b = function(a, b) {
			if (a & 1) {
				_.F(0, "div", 0), _.B(1, f7b, 2, 2, "button", 1)(2, g7b, 2, 4, "a", 1)(3, h7b, 2, 3, "a", 2)(4, i7b, 2, 3, "a", 3), _.H();
			}
			if (a & 2) {
				let c;
				a = _.K();
				_.y();
				_.C((c = b.kind) === a.Qda.Wjb ? 1 : c === a.Qda.PV ? 2 : c === a.Qda.aib ? 3 : c === a.Qda.Zhb ? 4 : -1);
			}
		};
		k7b = function(a) {
			if (a & 1) {
				_.F(0, "button", 0), _.I(1, "cm-icon", 3), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.E("icon", a.Wk.hzb);
			}
		};
		l7b = function(a) {
			if (a & 1) {
				_.I(0, "cfc-tooltip-action", 5);
			}
			if (a & 2) {
				a = _.K(2), _.E("action", a.Hd.a5);
			}
		};
		m7b = function(a) {
			if (a & 1) {
				_.R(0), _.z(1, l7b, 1, 1, "cfc-tooltip-action", 4);
			}
			if (a & 2) {
				a = _.K(), _.S(" ", a.Hd.content, " "), _.y(), _.E("ngIf", a.Hd.a5);
			}
		};
		n7b = function(a) {
			if (a & 1) {
				_.I(0, "cfc-tooltip-action", 5);
			}
			if (a & 2) {
				a = _.K(2), _.E("action", a.Hd.a5);
			}
		};
		o7b = function(a) {
			if (a & 1) {
				_.I(0, "span", 6), _.z(1, n7b, 1, 1, "cfc-tooltip-action", 4);
			}
			if (a & 2) {
				a = _.K(), _.E("innerHTML", a.Hd.content, _.qg), _.y(), _.E("ngIf", a.Hd.a5);
			}
		};
		p7b = function(a) {
			if (a & 1) {
				_.I(0, "div", 2);
			}
			if (a & 2) {
				a = _.K(), _.E("ngTemplateOutlet", a.Hd.content)("ngTemplateOutletContext", a.Hd.cQ);
			}
		};
		q7b = function(a, b) {
			if (a && b(a)) {
				q7b(a.parentNode, b);
			}
		};
		r7b = function(a) {
			var b = undefined;
			q7b(a, (c) => c.classList && c.classList.contains("cfc-remove-from-tab-order") ? (b = c, false) : true);
			return b;
		};
		t7b = function(a, b = false) {
			var c = [];
			a = a.querySelectorAll("[tabindex], a[href], area[href], iframe, input, textarea, select, button");
			for (let d = 0; d < a.length; ++d) {
				let e = a[d];
				if (s7b(e) || b) {
					c.push(e);
				}
			}
			return c;
		};
		s7b = function(a) {
			var b;
			if (!(b = !a.offsetParent || a.disabled)) {
				b = !!r7b(a);
			}
			return b ? false : (a = a.attributes.getNamedItem("tabindex")) && Number(a.value) === -1 ? false : true;
		};
		u7b = function(a, b) {
			var c = a.id;
			new _.ef((d) => {
				var e = new _.af();
				e.add(a.responses.pipe(G6b(c), _.uf(({ data: f }) => f)).subscribe(d));
				e.add(a.onComplete.pipe(G6b(c)).subscribe(() => undefined));
				e.add(a.onError.pipe(G6b(c)).subscribe(({ err: f }) => undefined));
				a.F.next({
					id: c,
					data: b,
					type: "REQ"
				});
				return () => undefined;
			}).pipe(_.lP()).connect();
		};
		v7b = class {
			constructor(a, b) {
				this.A = a;
				this.F = b;
				a = this.F.asObservable();
				this.requests = a.pipe(_.Gf((c) => c.type === "REQ"), _.Xg());
				this.responses = a.pipe(_.Gf((c) => c.type === "RES"), _.Xg());
				this.onComplete = a.pipe(_.Gf((c) => c.type === "COMPLETE"), _.Xg());
				this.onError = a.pipe(_.Gf((c) => c.type === "ERROR"), _.Xg());
			}
			get id() {
				var a = this.A.get() || 0;
				a = a >= Number.MAX_SAFE_INTEGER ? 0 : a + 1;
				this.A.set(a);
				return `${Date.now()}_${a}`;
			}
		};
		w7b = function(a, b) {
			var c = new _.yP(`${b}_id`);
			a = _.zP(a, `${b}_messageBus`);
			return new v7b(c, a);
		};
		x7b = [
			[
				[
					"",
					8,
					"material-icons",
					3,
					"iconPositionEnd",
					""
				],
				[
					"mat-icon",
					3,
					"iconPositionEnd",
					""
				],
				[
					"",
					"matButtonIcon",
					"",
					3,
					"iconPositionEnd",
					""
				]
			],
			"*",
			[
				[
					"",
					"iconPositionEnd",
					"",
					8,
					"material-icons"
				],
				[
					"mat-icon",
					"iconPositionEnd",
					""
				],
				[
					"",
					"matButtonIcon",
					"",
					"iconPositionEnd",
					""
				]
			],
			[[
				"",
				"progressIndicator",
				""
			]]
		];
		uQ = class extends _.VB {
			constructor() {
				super();
				this.Wl = _.m(_.kjb, { optional: true });
				this.v3 = true;
				this.Wl = this.Wl || _.jjb;
				this.color = this.Wl.color || _.jjb.color;
			}
		};
		uQ.J = function(a) {
			return new (a || uQ)();
		};
		uQ.ka = _.u({
			type: uQ,
			da: [
				[
					"button",
					"mat-mini-fab",
					""
				],
				[
					"a",
					"mat-mini-fab",
					""
				],
				[
					"button",
					"matMiniFab",
					""
				],
				[
					"a",
					"matMiniFab",
					""
				]
			],
			eb: [
				1,
				"mdc-fab",
				"mat-mdc-fab-base",
				"mdc-fab--mini",
				"mat-mdc-mini-fab"
			],
			Cc: ["matButton", "matAnchor"],
			features: [_.nh],
			fc: [
				".material-icons:not([iconPositionEnd]), mat-icon:not([iconPositionEnd]), [matButtonIcon]:not([iconPositionEnd])",
				"*",
				".material-icons[iconPositionEnd], mat-icon[iconPositionEnd], [matButtonIcon][iconPositionEnd]",
				"[progressIndicator]"
			],
			ha: 8,
			ia: 5,
			la: [
				[1, "mat-mdc-button-persistent-ripple"],
				[1, "mdc-button__label"],
				[1, "mat-mdc-button-progress-indicator-container"],
				[1, "mat-focus-indicator"],
				[1, "mat-mdc-button-touch-target"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Xh(x7b), _.Fh(0, "span", 0), _.Yh(1), _.Dh(2, "span", 1), _.Yh(3, 1), _.Eh(), _.Yh(4, 2), _.B(5, I6b, 2, 0, "div", 2), _.Fh(6, "span", 3)(7, "span", 4);
				}
				if (a & 2) {
					_.P("mdc-button__ripple", !b.v3)("mdc-fab__ripple", b.v3), _.y(5), _.C(b.Wba() ? 5 : -1);
				}
			},
			styles: [".mat-mdc-fab-base{-webkit-user-select:none;user-select:none;position:relative;display:inline-flex;align-items:center;justify-content:center;box-sizing:border-box;width:56px;height:56px;padding:0;border:none;fill:currentColor;text-decoration:none;cursor:pointer;-moz-appearance:none;-webkit-appearance:none;overflow:visible;transition:box-shadow 280ms cubic-bezier(0.4, 0, 0.2, 1),opacity 15ms linear 30ms,transform 270ms 0ms cubic-bezier(0, 0, 0.2, 1);flex-shrink:0;-webkit-tap-highlight-color:rgba(0,0,0,0)}.mat-mdc-fab-base .mat-mdc-button-ripple,.mat-mdc-fab-base .mat-mdc-button-persistent-ripple,.mat-mdc-fab-base .mat-mdc-button-persistent-ripple::before{top:0;left:0;right:0;bottom:0;position:absolute;pointer-events:none;border-radius:inherit}.mat-mdc-fab-base .mat-mdc-button-ripple{overflow:hidden}.mat-mdc-fab-base .mat-mdc-button-persistent-ripple::before{content:\"\";opacity:0}.mat-mdc-fab-base .mdc-button__label,.mat-mdc-fab-base .mat-icon{z-index:1;position:relative}.mat-mdc-fab-base .mat-focus-indicator{top:0;left:0;right:0;bottom:0;position:absolute}.mat-mdc-fab-base:focus-visible>.mat-focus-indicator::before{content:\"\"}.mat-mdc-fab-base._mat-animation-noopable{transition:none !important;animation:none !important}.mat-mdc-fab-base::before{position:absolute;box-sizing:border-box;width:100%;height:100%;top:0;left:0;border:1px solid rgba(0,0,0,0);border-radius:inherit;content:\"\";pointer-events:none}.mat-mdc-fab-base[hidden]{display:none}.mat-mdc-fab-base::-moz-focus-inner{padding:0;border:0}.mat-mdc-fab-base:active,.mat-mdc-fab-base:focus{outline:none}.mat-mdc-fab-base:hover{cursor:pointer}.mat-mdc-fab-base>svg{width:100%}.mat-mdc-fab-base .mat-icon,.mat-mdc-fab-base .material-icons{transition:transform 180ms 90ms cubic-bezier(0, 0, 0.2, 1);fill:currentColor;will-change:transform}.mat-mdc-fab-base .mat-focus-indicator::before{margin:calc(calc(var(--mat-focus-indicator-border-width, 3px) + 2px)*-1)}.mat-mdc-fab-base[disabled],.mat-mdc-fab-base.mat-mdc-button-disabled{cursor:default;pointer-events:none}.mat-mdc-fab-base[disabled],.mat-mdc-fab-base[disabled]:focus,.mat-mdc-fab-base.mat-mdc-button-disabled,.mat-mdc-fab-base.mat-mdc-button-disabled:focus{box-shadow:none}.mat-mdc-fab-base.mat-mdc-button-disabled-interactive{pointer-events:auto}.mat-mdc-fab{background-color:var(--mat-fab-container-color, var(--mat-sys-primary-container));border-radius:var(--mat-fab-container-shape, var(--mat-sys-corner-large));color:var(--mat-fab-foreground-color, var(--mat-sys-on-primary-container, inherit));box-shadow:var(--mat-fab-container-elevation-shadow, var(--mat-sys-level3))}@media(hover: hover){.mat-mdc-fab:hover{box-shadow:var(--mat-fab-hover-container-elevation-shadow, var(--mat-sys-level4))}}.mat-mdc-fab:focus{box-shadow:var(--mat-fab-focus-container-elevation-shadow, var(--mat-sys-level3))}.mat-mdc-fab:active,.mat-mdc-fab:focus:active{box-shadow:var(--mat-fab-pressed-container-elevation-shadow, var(--mat-sys-level3))}.mat-mdc-fab[disabled],.mat-mdc-fab.mat-mdc-button-disabled{cursor:default;pointer-events:none;color:var(--mat-fab-disabled-state-foreground-color, color-mix(in srgb, var(--mat-sys-on-surface) 38%, transparent));background-color:var(--mat-fab-disabled-state-container-color, color-mix(in srgb, var(--mat-sys-on-surface) 12%, transparent))}.mat-mdc-fab.mat-mdc-button-disabled-interactive{pointer-events:auto}.mat-mdc-fab .mat-mdc-button-touch-target{position:absolute;top:50%;height:var(--mat-fab-touch-target-size, 48px);display:var(--mat-fab-touch-target-display, block);left:50%;width:var(--mat-fab-touch-target-size, 48px);transform:translate(-50%, -50%)}.mat-mdc-fab .mat-ripple-element{background-color:var(--mat-fab-ripple-color, color-mix(in srgb, var(--mat-sys-on-primary-container) calc(var(--mat-sys-pressed-state-layer-opacity) * 100%), transparent))}.mat-mdc-fab .mat-mdc-button-persistent-ripple::before{background-color:var(--mat-fab-state-layer-color, var(--mat-sys-on-primary-container))}.mat-mdc-fab.mat-mdc-button-disabled .mat-mdc-button-persistent-ripple::before{background-color:var(--mat-fab-disabled-state-layer-color)}.mat-mdc-fab:hover>.mat-mdc-button-persistent-ripple::before{opacity:var(--mat-fab-hover-state-layer-opacity, var(--mat-sys-hover-state-layer-opacity))}.mat-mdc-fab.cdk-program-focused>.mat-mdc-button-persistent-ripple::before,.mat-mdc-fab.cdk-keyboard-focused>.mat-mdc-button-persistent-ripple::before,.mat-mdc-fab.mat-mdc-button-disabled-interactive:focus>.mat-mdc-button-persistent-ripple::before{opacity:var(--mat-fab-focus-state-layer-opacity, var(--mat-sys-focus-state-layer-opacity))}.mat-mdc-fab:active>.mat-mdc-button-persistent-ripple::before{opacity:var(--mat-fab-pressed-state-layer-opacity, var(--mat-sys-pressed-state-layer-opacity))}.mat-mdc-mini-fab{width:40px;height:40px;background-color:var(--mat-fab-small-container-color, var(--mat-sys-primary-container));border-radius:var(--mat-fab-small-container-shape, var(--mat-sys-corner-medium));color:var(--mat-fab-small-foreground-color, var(--mat-sys-on-primary-container, inherit));box-shadow:var(--mat-fab-small-container-elevation-shadow, var(--mat-sys-level3))}@media(hover: hover){.mat-mdc-mini-fab:hover{box-shadow:var(--mat-fab-small-hover-container-elevation-shadow, var(--mat-sys-level4))}}.mat-mdc-mini-fab:focus{box-shadow:var(--mat-fab-small-focus-container-elevation-shadow, var(--mat-sys-level3))}.mat-mdc-mini-fab:active,.mat-mdc-mini-fab:focus:active{box-shadow:var(--mat-fab-small-pressed-container-elevation-shadow, var(--mat-sys-level3))}.mat-mdc-mini-fab[disabled],.mat-mdc-mini-fab.mat-mdc-button-disabled{cursor:default;pointer-events:none;color:var(--mat-fab-small-disabled-state-foreground-color, color-mix(in srgb, var(--mat-sys-on-surface) 38%, transparent));background-color:var(--mat-fab-small-disabled-state-container-color, color-mix(in srgb, var(--mat-sys-on-surface) 12%, transparent))}.mat-mdc-mini-fab.mat-mdc-button-disabled-interactive{pointer-events:auto}.mat-mdc-mini-fab .mat-mdc-button-touch-target{position:absolute;top:50%;height:var(--mat-fab-small-touch-target-size, 48px);display:var(--mat-fab-small-touch-target-display);left:50%;width:var(--mat-fab-small-touch-target-size, 48px);transform:translate(-50%, -50%)}.mat-mdc-mini-fab .mat-ripple-element{background-color:var(--mat-fab-small-ripple-color, color-mix(in srgb, var(--mat-sys-on-primary-container) calc(var(--mat-sys-pressed-state-layer-opacity) * 100%), transparent))}.mat-mdc-mini-fab .mat-mdc-button-persistent-ripple::before{background-color:var(--mat-fab-small-state-layer-color, var(--mat-sys-on-primary-container))}.mat-mdc-mini-fab.mat-mdc-button-disabled .mat-mdc-button-persistent-ripple::before{background-color:var(--mat-fab-small-disabled-state-layer-color)}.mat-mdc-mini-fab:hover>.mat-mdc-button-persistent-ripple::before{opacity:var(--mat-fab-small-hover-state-layer-opacity, var(--mat-sys-hover-state-layer-opacity))}.mat-mdc-mini-fab.cdk-program-focused>.mat-mdc-button-persistent-ripple::before,.mat-mdc-mini-fab.cdk-keyboard-focused>.mat-mdc-button-persistent-ripple::before,.mat-mdc-mini-fab.mat-mdc-button-disabled-interactive:focus>.mat-mdc-button-persistent-ripple::before{opacity:var(--mat-fab-small-focus-state-layer-opacity, var(--mat-sys-focus-state-layer-opacity))}.mat-mdc-mini-fab:active>.mat-mdc-button-persistent-ripple::before{opacity:var(--mat-fab-small-pressed-state-layer-opacity, var(--mat-sys-pressed-state-layer-opacity))}.mat-mdc-extended-fab{-moz-osx-font-smoothing:grayscale;-webkit-font-smoothing:antialiased;padding-left:20px;padding-right:20px;width:auto;max-width:100%;line-height:normal;box-shadow:var(--mat-fab-extended-container-elevation-shadow, var(--mat-sys-level3));height:var(--mat-fab-extended-container-height, 56px);border-radius:var(--mat-fab-extended-container-shape, var(--mat-sys-corner-large));font-family:var(--mat-fab-extended-label-text-font, var(--mat-sys-label-large-font));font-size:var(--mat-fab-extended-label-text-size, var(--mat-sys-label-large-size));font-weight:var(--mat-fab-extended-label-text-weight, var(--mat-sys-label-large-weight));letter-spacing:var(--mat-fab-extended-label-text-tracking, var(--mat-sys-label-large-tracking))}@media(hover: hover){.mat-mdc-extended-fab:hover{box-shadow:var(--mat-fab-extended-hover-container-elevation-shadow, var(--mat-sys-level4))}}.mat-mdc-extended-fab:focus{box-shadow:var(--mat-fab-extended-focus-container-elevation-shadow, var(--mat-sys-level3))}.mat-mdc-extended-fab:active,.mat-mdc-extended-fab:focus:active{box-shadow:var(--mat-fab-extended-pressed-container-elevation-shadow, var(--mat-sys-level3))}.mat-mdc-extended-fab[disabled],.mat-mdc-extended-fab.mat-mdc-button-disabled{cursor:default;pointer-events:none}.mat-mdc-extended-fab[disabled],.mat-mdc-extended-fab[disabled]:focus,.mat-mdc-extended-fab.mat-mdc-button-disabled,.mat-mdc-extended-fab.mat-mdc-button-disabled:focus{box-shadow:none}.mat-mdc-extended-fab.mat-mdc-button-disabled-interactive{pointer-events:auto}[dir=rtl] .mat-mdc-extended-fab .mdc-button__label+.mat-icon,[dir=rtl] .mat-mdc-extended-fab .mdc-button__label+.material-icons,.mat-mdc-extended-fab>.mat-icon,.mat-mdc-extended-fab>.material-icons{margin-left:-8px;margin-right:12px}.mat-mdc-extended-fab .mdc-button__label+.mat-icon,.mat-mdc-extended-fab .mdc-button__label+.material-icons,[dir=rtl] .mat-mdc-extended-fab>.mat-icon,[dir=rtl] .mat-mdc-extended-fab>.material-icons{margin-left:12px;margin-right:-8px}.mat-mdc-extended-fab .mat-mdc-button-touch-target{width:100%}.mat-mdc-button-progress-indicator-container{position:absolute;inset-inline-start:0;margin-block-start:0;display:flex;align-items:center;justify-content:center;width:100%;height:100%;box-sizing:border-box}.mat-mdc-button-progress-indicator-shown mat-icon,.mat-mdc-button-progress-indicator-shown [matButtonIcon],.mat-mdc-button-progress-indicator-shown .mdc-button__label{visibility:hidden}\n"],
			Ab: 2
		});
		var y7b = function(a) {
			var b;
			return _.e1b(_.a4b) ? null : (b = a.F) == null ? undefined : _.ek(b, 1, _.gb);
		};
		var vQ = class {
			constructor(a) {
				this.F = null;
				_.m(_.Cl);
				this.location = _.m(_.gm);
				this.A = a.A ? (0, _.d4b)(a.A) : new _.MP();
				this.Fl = !!_.ek(this.A, 2, _.gb);
				if (_.ek(this.A, 2, _.gb)) {
					this.F = _.Z(this.A, _.Z3b, 1, _.gb);
				}
			}
		};
		vQ.J = function(a) {
			return new (a || vQ)(_.ae(_.JP), _.ae(_.e4b));
		};
		vQ.sa = _.Cd({
			token: vQ,
			factory: vQ.J,
			wa: "root"
		});
		var z7b = class {
			constructor(a) {
				this.Jb = a;
				new _.yP(rQ`clearcut`);
				this.A = new _.yP(rQ`debugLoggers`);
				if (this.A.get() === undefined) {
					this.A.set([]);
				}
				this.U = w7b(this.Jb, rQ`trackPageView`);
				this.I = w7b(this.Jb, rQ`sendToClearcut`);
				this.H = w7b(this.Jb, rQ`sendEvent`);
				this.R = w7b(this.Jb, rQ`sendToGtm`);
			}
		};
		a: {
			let a;
			try {
				let b = top.__rif_debug;
				if (b) {
					A7b = b;
					break a;
				}
				a = top;
			} catch (b) {
				a = window;
			}
			A7b = a.__rif_debug = {
				host: {},
				sandboxes: new WeakMap()
			};
		}
		if (!_.aP()) {
			B7b.host.sandboxRegistry = {
				ska: C7b,
				G9b: D7b,
				Y6b: N6b,
				X6b: O6b
			};
		}
		new _.he("SANDBOX_TRACKER");
		if (parent === window) {
			B7b.host.setDelayForUnloadMs = () => {}, B7b.host.setKeepAliveMax = () => {};
		}
		_.xQ = class extends z7b {
			constructor(a, b) {
				super(b);
				this.F = a.F;
			}
			trackPageView(a) {
				u7b(this.U, { latencyMillis: a });
			}
			sendEvent(a, b = "concordEvent") {
				u7b(this.H, {
					analyticsEvent: a,
					gtmEventTitle: b
				});
			}
			sendToClearcut(a, b) {
				u7b(this.I, {
					concordEvent: a,
					eventTimeMs: b
				});
			}
			sendZeroStatePageEvent(a) {
				this.sendEvent({
					type: "gettingStarted",
					name: "showZeroState",
					metadata: a
				});
			}
			recordExperimentExposure(a, b = false, c = true) {
				if (a) {
					b ? this.sendEvent({
						name: String(a),
						type: "experimentExposure",
						metadata: { gtmActiveExperiments: this.F.toString() },
						clearcutOnly: c
					}) : this.sendEvent({
						name: String(a),
						type: "experimentExposure",
						clearcutOnly: c
					});
				}
			}
			recordAutomatedExperimentExposure(a) {
				if (a) {
					a.key !== undefined ? this.sendEvent({
						type: "automatedExperimentExposure",
						name: String(a.key),
						clearcutOnly: true
					}) : this.sendEvent({
						type: "automatedExperimentExposure",
						name: a,
						clearcutOnly: true
					});
				}
			}
			sendEventOnElementClick(a) {
				var b = a.getAttribute("track-type");
				var c = a.getAttribute("track-name");
				if (b && c) {
					let d = {};
					for (let e = 0; e < a.attributes.length; e++) {
						let f = a.attributes[e].name;
						let g = a.attributes[e].value;
						if (f.indexOf("track-metadata-") !== -1) {
							f = f.substring(15), (f = _.kRa(f)) && g && (d[f] = g);
						}
					}
					this.sendEvent({
						type: b,
						name: c,
						metadata: d
					});
				}
			}
			registerDebugLogger(a) {
				this.A.get().push(a);
			}
			setConfig(a) {
				u7b(this.R, { gtmEvent: a });
			}
			getActivatedExperimentsVariants() {
				return new Set(this.F);
			}
		};
		_.xQ.J = function(a) {
			return new (a || _.xQ)(_.ae(_.JP), _.ae(_.AP));
		};
		_.xQ.sa = _.Cd({
			token: _.xQ,
			factory: _.xQ.J,
			wa: "root"
		});
		var sQ = class extends _.Jk {
			constructor() {
				super(_.KP.instance ? _.KP.instance : new _.KP(), new _.Bcb());
				if (_.aP()) throw Error("Qf");
				if (sQ.instance) return sQ.instance;
				sQ.instance = this;
			}
		};
		sQ.J = function(a) {
			return new (a || sQ)();
		};
		sQ.sa = _.Cd({
			token: sQ,
			factory: sQ.J,
			wa: "root"
		});
		var F7b = (E7b = Symbol.toStringTag, Symbol.iterator);
		var H7b = function(a, b) {
			if (a.wFa.has(b)) {
				var c = a.Ow.zf.pipe(_.Gf((g) => g.action === "SET" && g.key === b), _.uf((g) => g.value));
				var d = a.wFa.get(b).pipe(_.eh((g) => {
					if (!a.Ow.has(b)) {
						var k = a.tV.get(b);
						a.Ow.set(b, g, k ? { OZa: k.PCb } : {});
					}
				}));
				var e;
				var f = _.mf(null).pipe(_.ch(() => a.xB && a.xB.get(b) ? _.Ff(a.xB.sT(b), _.Df(1e3)).pipe(_.uf(() => a.xB.get(b)), _.Gf((g) => g === undefined), _.Qg(), _.uf(() => null)) : _.mf(null)), _.eh(() => {
					e = G7b(a, b);
				}), _.ch(() => {
					var g = a.Ow.get(b);
					return g === undefined ? _.s1b(c, d).pipe(_.Qg()) : _.mf(g);
				}), _.eh(() => {
					var g = e;
					if (a.xB) {
						g.unsubscribe(), a.xB.delete(b);
					}
				}, () => {
					var g = e;
					if (a.xB) {
						g.unsubscribe(), a.xB.delete(b);
					}
				}), _.Yg());
				a.TB.set(b, f);
			}
		};
		var I7b = function(a, b, c) {
			return c === undefined ? false : c.NZa && !c.NZa() ? (a.delete(b), true) : false;
		};
		var J7b = function(a) {
			for (let [b, c] of a.tV) I7b(a, b, c);
		};
		var K7b = class {
			constructor(a, b = {}, c) {
				this.TB = a;
				this.Lga = b;
				this.FIb = c;
				this.xB = this.Ow = undefined;
				this[E7b] = "Cache";
				this.tV = new Map();
				this.wFa = new Map();
				this.zf = _.Af(this.TB, "change").pipe(_.uf((d) => ({
					key: d.key,
					value: this.yI(d.value),
					action: d.type
				})));
				if (this.Ow) {
					this.Ow.zf.pipe(_.Gf((d) => d.action === "REMOVE")).subscribe((d) => {
						H7b(this, d.key);
					});
				}
			}
			get(a, b) {
				if (!this.has(a) && b) return this.set(a, b.n8b(), b.options), this.get(a);
				if (this.Ow) {
					this.Ow.has(a);
				}
				b = this.tV.get(a);
				if (!b || !I7b(this, a, b)) return a = this.TB.get(a), a === undefined ? undefined : this.yI(a);
			}
			has(a) {
				return this.TB.has(a);
			}
			set(a, b, c = {}) {
				if (c.byb != null && this.FIb) throw Error("Rf");
				if (this.Ow) {
					if (this.has(a)) throw Error("Sf");
					if (!(b instanceof _.ef)) throw Error("Tf");
					this.wFa.set(a, b);
					H7b(this, a);
				} else {
					b = this.Lga.Wn ? this.Lga.Wn.serialize(b) : b;
					this.TB.set(a, b, c.OZa);
				}
				this.tV.delete(a);
				if (Object.keys(c).length > 0) {
					this.tV.set(a, {
						PCb: c.OZa,
						NZa: c.byb
					});
				}
				return this;
			}
			delete(a) {
				if (this.Ow) return this.Ow.delete(a);
				var b = this.TB.delete(a);
				if (b) {
					this.tV.delete(a);
				}
				return b;
			}
			clear() {
				if (this.Ow) {
					this.Ow.clear();
				} else {
					this.tV.clear(), this.TB.clear();
				}
			}
			get size() {
				J7b(this);
				return this.TB.size;
			}
			values() {
				var a = this;
				return function* () {
					J7b(a);
					for (let b of a.TB.values()) yield a.yI(b);
				}();
			}
			keys() {
				J7b(this);
				return this.TB.keys();
			}
			entries() {
				var a = this;
				return function* () {
					J7b(a);
					for (let [b, c] of a.TB.entries()) yield [b, a.yI(c)];
				}();
			}
			[F7b]() {
				return this.entries();
			}
			forEach(a, b) {
				for (let [c, d] of this.entries()) a.call(b, d, c, this);
			}
			sT(a) {
				return this.zf.pipe(_.Gf((b) => b.key === a), _.uf((b) => () => b.action === "REMOVE" ? undefined : b.value), _.bh(() => this.get(a)), _.uf((b) => b()));
			}
			yI(a) {
				return this.Lga.Wn ? this.Lga.Wn.deserialize(a) : a;
			}
		};
		var yQ = class {
			constructor(a) {
				this.globalCacheDataService = a;
			}
			create(a, b, c = {}) {
				b = this.globalCacheDataService.getOrCreateListenableMap(a, c, undefined);
				return new K7b(b, c, a !== "NON_SHAREABLE_CACHE_ID");
			}
		};
		yQ.J = function(a) {
			return new (a || yQ)(_.ae(_.wP));
		};
		yQ.sa = _.Cd({
			token: yQ,
			factory: yQ.J,
			wa: "root"
		});
		var L7b = ["click"];
		var M7b = function(a, b) {
			if (_.Mv(b.target, (c) => c ? a.A.has(c) && c instanceof Element && c.getAttribute("aria-disabled") === "true" : false)) {
				b.preventDefault(), b.stopPropagation();
			}
		};
		var zQ = class {
			constructor() {
				this.F = [];
				this.A = new Set();
				var a = _.m(_.Xk);
				for (let b of L7b) this.F.push(_.Mp(a, b, (c) => {
					M7b(this, c);
				}, { capture: true }));
			}
			Ba() {
				for (let a of this.F) _.Xv(a);
			}
		};
		zQ.J = function(a) {
			return new (a || zQ)();
		};
		zQ.sa = _.Cd({
			token: zQ,
			factory: zQ.J,
			wa: "root"
		});
		new _.he("CM_BUTTON_ALLOW_EXTENDED_TYPES");
		_.N7b = new _.he("CM3_BUTTON_BLOCK_EXTENDED_TYPES");
		var O7b = new _.he("CM_BUTTON_ALLOW_FLAT_EXTENDED_TYPES");
		_.P7b = new _.he("CM_BUTTON_CONFIG");
		var AQ = class {
			get disabled() {
				return this.A;
			}
			set disabled(a) {
				this.A = a;
			}
			constructor(a, b) {
				this.Ga = a;
				this.ma = b;
				this.A = this.destroyed = false;
			}
			Rb() {
				_.S6b(this.Ga.nativeElement);
			}
			Ba() {
				this.destroyed = true;
			}
			focus() {
				this.Ga.nativeElement.focus();
			}
			zJa() {}
			UY() {
				return this.Ga;
			}
			eE() {
				return this.Ga;
			}
		};
		AQ.J = function(a) {
			return new (a || AQ)(_.Dg(_.Jf), _.Dg(zQ));
		};
		AQ.Oa = _.We({ type: AQ });
		var Q7b = new _.he("Cloud Matter theme token");
		_.BQ = class {
			constructor(a) {
				_.m(_.xQ);
				var b = _.m(_.AP);
				this.H = a != null ? a : "none";
				this.A = new _.ml(this.H);
				this.X = this.A.pipe(_.Sg());
				this.aa = this.A.pipe(_.uf(() => ""), _.Sg());
				this.I = _.zP(b, "DarkModeEnablementSubjectId", {
					variant: "BEHAVIOR_SUBJECT",
					initialValue: false
				});
				this.F = _.zP(b, "ContrastModeEnablementSubjectId", {
					variant: "BEHAVIOR_SUBJECT",
					initialValue: "default"
				});
				this.U = this.I.asObservable().pipe(_.Sg());
				this.R = _.Ck(this.A, { initialValue: this.H });
				_.Ck(this.aa, { initialValue: "" });
				_.Ck(this.U, { initialValue: this.I.PDa });
				_.Ck(this.F.asObservable().pipe(_.Sg()), { initialValue: this.F.PDa });
				a = _.m(_.RP).IY();
				new BroadcastChannel(`user-preference-color-setting${a}`);
				new BroadcastChannel(`user-preference-contrast-setting${a}`);
			}
			get theme() {
				return this.A.getValue();
			}
			set theme(a) {
				this.A.next(a);
			}
		};
		_.BQ.J = function(a) {
			return new (a || _.BQ)(_.ae(Q7b, 8));
		};
		_.BQ.sa = _.Cd({
			token: _.BQ,
			factory: _.BQ.J,
			wa: "root"
		});
		var R7b = new _.he("DisableHost");
		var S7b = "cfc-labelledby-message-container-" + _.iu();
		var CQ = new Map();
		var DQ = null;
		var U6b = _.m4b();
		var T7b = function(a, b) {
			a = tQ(a);
			b = (b = CQ.get(b)) && b.Ip.id;
			return !!b && a.includes(b);
		};
		var U7b = function(a, b, c, d) {
			var e = X6b(c, d);
			if (typeof c !== "string") {
				T6b(c);
				CQ.set(e, {
					Ip: c,
					Cv: 0
				});
			} else if (!CQ.has(e)) {
				let f = a.document.createElement("div");
				T6b(f);
				f.textContent = c;
				f.setAttribute("aria-hidden", "false");
				if (d) {
					f.setAttribute("role", d);
				}
				if (!DQ) {
					let g = a.document.getElementById(S7b);
					if (g) {
						g.parentNode.removeChild(g);
					}
					DQ = a.document.createElement("div");
					DQ.id = S7b;
					DQ.style.display = "none";
					a.document.body.appendChild(DQ);
				}
				DQ.appendChild(f);
				CQ.set(X6b(c, d), {
					Ip: f,
					Cv: 0
				});
			}
			if (!T7b(b, e) && b) {
				a = CQ.get(e), V6b(b, a.Ip.id), b.setAttribute("cfc-labelledby-host", ""), a.Cv++;
			}
		};
		var V7b = function(a, b) {
			if (a) {
				b = CQ.get(b);
				b.Cv--;
				W6b(a, b.Ip.id);
				var c = tQ(a);
				if (!Array.from(CQ.values()).some(({ Ip: d }) => c.includes(d.id))) {
					a.removeAttribute("cfc-labelledby-host");
				}
			}
		};
		var W7b = function() {
			if (DQ && DQ.parentNode) {
				DQ.parentNode.removeChild(DQ), DQ = null;
			}
		};
		var X7b = function(a, b, c) {
			c = X6b(b, c);
			if (T7b(a, c)) {
				V7b(a, c);
			}
			if (typeof b === "string" && (a = CQ.get(c)) && a.Cv === 0) {
				a = (a = CQ.get(c)) && a.Ip, DQ && a && DQ.removeChild(a), CQ.delete(c);
			}
			if (DQ && DQ.childNodes.length === 0) {
				W7b();
			}
		};
		var Y7b = function(a) {
			if (a) {
				var b = tQ(a).filter((c) => !c.startsWith("cfc-labelledby-message"));
				a.setAttribute("aria-labelledby", b.join(" "));
			}
		};
		var EQ = class {
			constructor() {
				this.document = _.m(_.Xk);
			}
			Ba() {
				var a = this.document.querySelectorAll("[cfc-labelledby-host]");
				for (let b of a) Y7b(b), b.removeAttribute("cfc-labelledby-host");
				if (DQ) {
					W7b();
				}
				CQ.clear();
			}
		};
		EQ.J = function(a) {
			return new (a || EQ)();
		};
		EQ.sa = _.Cd({
			token: EQ,
			factory: EQ.J,
			wa: "root"
		});
		var Z7b = _.cP(new _.hP("45405343")) ? 400 : 0;
		var $7b = new _.he("TOOLTIP_POSITION");
		var a8b = new _.he("TOOLTIP_VISIBLE_ON_OVERLAY_HOVER");
		var b8b = new _.he("TOOLTIP_MOCK_REQUEST_ANIMATION_FRAME");
		var c8b = new _.he("TOOLTIP_RENDER_DEBOUNCE_MS");
		var d8b = class {
			constructor() {
				this.content = "";
				this.ot = false;
				this.LVa = "Disabled explanation";
				this.b5 = false;
				this.c5 = "bottomleft";
				this.bha = false;
				this.Hwa = Z7b;
				this.MVa = true;
				this.KVa = false;
				this.Fwa = true;
				this.d5 = false;
				this.cQ = {};
				this.zf = new _.Wg();
				this.eC = new _.Wg();
				this.ZB = new _.Wg();
			}
			set jw(a) {
				if (a instanceof _.Zh || a instanceof _.od || typeof a === "string") this.content = a;
				else throw Error("Uf");
			}
			get jw() {
				return this.content;
			}
			get nq() {
				return this.Ga.nativeElement ? this.Ga.nativeElement.tagName === "CFC-ICON" || this.Ga.nativeElement.tagName === "CM-ICON" || this.Ga.nativeElement.hasAttribute("mat-icon-button") ? this.A ? "description" : "label" : "description" : "label";
			}
			get A() {
				return this.Ga.nativeElement.getAttribute("aria-label");
			}
		};
		var e8b = class {
			constructor(a) {
				this.Nb = a;
				this.A = new Map();
			}
		};
		var g8b = function(a, b) {
			var c = b.Nb.bj;
			var d = _.em(a.F);
			var e;
			var f;
			a = [...Array.from(((e = d.parentElement) == null ? undefined : e.children) || []), ...Array.from(((f = c.parentElement) == null ? undefined : f.children) || [])];
			if (e = f8b()) {
				a.push(...Array.from(e.children));
			}
			a.forEach((g) => {
				if (g !== d && g.nodeName !== "SCRIPT" && !g.classList.contains("cfc-accessible-modal-overlay-do-not-hide") && g.nodeName !== "STYLE" && !g.hasAttribute("aria-live")) {
					var k = g.getAttribute("aria-hidden");
					b.A.set(g, k === "true" ? true : false);
					if (g === c) {
						g.removeAttribute("aria-hidden");
					} else {
						g.setAttribute("aria-hidden", "true");
					}
				}
			});
		};
		var h8b = function(a, b) {
			b = new e8b(b);
			g8b(a, b);
			a.A.push(b);
		};
		var i8b = function(a) {
			a.A.forEach((b, c) => {
				if (b) {
					c.setAttribute("aria-hidden", "true");
				} else {
					c.removeAttribute("aria-hidden");
				}
			});
		};
		var FQ = class {
			constructor() {
				this.F = _.m(_.Wl);
				this.A = [];
			}
		};
		FQ.J = function(a) {
			return new (a || FQ)();
		};
		FQ.sa = _.Cd({
			token: FQ,
			factory: FQ.J,
			wa: "root"
		});
		var GQ = class {
			constructor() {
				this.Ga = _.m(_.Jf);
			}
			Rb() {
				_.S6b(this.Ga.nativeElement);
			}
		};
		GQ.J = function(a) {
			return new (a || GQ)();
		};
		GQ.Oa = _.We({
			type: GQ,
			da: [[
				"",
				"cmMatFocusIndicator",
				""
			]]
		});
		var HQ = class {};
		HQ.J = function(a) {
			return new (a || HQ)();
		};
		HQ.sa = _.Cd({
			token: HQ,
			factory: HQ.J,
			wa: "root"
		});
		var IQ = class {
			constructor(a) {
				this.F = a;
				this.A = new WeakMap();
			}
		};
		IQ.J = function(a) {
			return new (a || IQ)(_.ae(_.Xf), _.ae(HQ));
		};
		IQ.sa = _.Cd({
			token: IQ,
			factory: IQ.J,
			wa: "root"
		});
		var l8b = function(a) {
			if (typeof Zone !== "undefined") {
				Zone.current.scheduleMacroTask("LoadModuleFactory", () => {}, {}, function(b) {
					a.finally(() => {
						b.invoke();
					});
				});
			}
		};
		var m8b = function(a) {
			var b = new Promise((c, d) => {
				_.gr().load(a).then(c, d);
			});
			l8b(b);
			return b;
		};
		var n8b = class {
			load(a) {
				return m8b(a).then(() => {
					var b = _.CTa.get(a);
					if (!b) throw new _.Sd(920, false);
					return new _.sva(b);
				});
			}
		};
		var JQ = class {
			constructor(a, b) {
				this.F = a;
				this.A = b;
				this.H = _.gr();
			}
		};
		JQ.J = function(a) {
			return new (a || JQ)(_.ae(n8b), _.ae(IQ));
		};
		JQ.sa = _.Cd({
			token: JQ,
			factory: JQ.J
		});
		var t8b = function(a) {
			if (!a.A) {
				a.yb.runOutsideAngular(() => {
					a.A = new MutationObserver(() => {
						s8b(a);
					});
				});
			}
		};
		var s8b = function(a) {
			if (a.F && !a.F.nativeElement.children.length) {
				var b = a.F.nativeElement.childNodes;
				if (b = b[b.length - 1]) {
					a = a.trimWhitespace(b.textContent);
					if (a !== b.textContent) {
						b.textContent = a;
					}
				}
			}
		};
		var KQ = class {
			get Zga() {
				return this.H;
			}
			set Zga(a) {
				if (this.H = a) {
					a.startsWith("URL__") || a.startsWith("CONSOLE__") || a.startsWith("CLOUD_DOCS__") || a.startsWith("SUPPORT__") || (a = `CONSOLE__${a}`);
				}
				this.R.then((b) => {
					b = b.getUrl(a);
					if (new _.hk(b).ug().endsWith(".google.com")) {
						let c = y7b(this.U);
						b = Y6b(b, c === "0" ? null : c);
					}
					_.nd(this.Ga.nativeElement, b);
				});
			}
			set kFa(a) {
				if (this.F = a) {
					t8b(this);
					this.A.observe(a.nativeElement, {
						characterData: true,
						subtree: true
					});
					s8b(this);
				}
			}
			constructor() {
				this.U = _.m(vQ);
				this.Ga = _.m(_.Jf);
				this.I = _.m(_.xQ);
				this.Xc = _.m(JQ);
				this.yb = _.m(_.th);
				this.icon = _.k4b;
				this.JT = _.Li(false, Object.assign({}, {}, { alias: "cfcDocHrefPreventDanglingIcon" }));
				this.R = q8b(this.Xc);
				this.H = "";
				this.Tmb = "external, opens new window";
				R6b(this.Ga.nativeElement, 55359);
			}
			onClick() {
				this.I.sendEvent({
					type: "docLink",
					name: "clickLink",
					metadata: { linkId: this.H }
				});
			}
			Rb() {
				this.Ga.nativeElement.setAttribute("aria-label", this.Hw());
			}
			Ba() {
				if (this.A) {
					this.A.disconnect();
				}
			}
			Hw(a) {
				var b = this.Ga.nativeElement.getAttribute("aria-label");
				return `${a || b || this.Ga.nativeElement.textContent.trim()} external, opens new window`;
			}
			trimWhitespace(a) {
				return a.replace(/\s+$/, "");
			}
		};
		KQ.J = function(a) {
			return new (a || KQ)();
		};
		KQ.ka = _.u({
			type: KQ,
			da: [[
				"a",
				"cfcDocHref",
				"",
				3,
				"mat-button",
				"",
				3,
				"mat-icon-button",
				"",
				3,
				"mat-raised-button",
				"",
				3,
				"mat-stroked-button",
				"",
				3,
				"mat-flat-button",
				""
			]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ci(r8b, 5);
				}
				if (a & 2) {
					let c;
					if (_.ei(c = _.fi())) {
						b.kFa = c.first;
					}
				}
			},
			eb: [
				"target",
				"_blank",
				"rel",
				"noopener",
				1,
				"cfc-doc-href"
			],
			Ua: 2,
			Ja: function(a, b) {
				if (a & 1) {
					_.J("click", function() {
						return b.onClick();
					});
				}
				if (a & 2) {
					_.P("cfc-doc-href-prevent-dangling-icon", b.JT());
				}
			},
			inputs: {
				Zga: "cfcDocHref",
				JT: [
					1,
					"cfcDocHrefPreventDanglingIcon",
					"preventDanglingIcon"
				]
			},
			fc: ["*"],
			ha: 6,
			ia: 3,
			la: [
				["ngContent", ""],
				[1, "cfc-doc-href-content"],
				[
					"role",
					"img",
					1,
					"cm-icon-external-link",
					3,
					"icon"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Xh(), _.F(0, "span", 1, 0), _.Yh(2), _.H(), _.B(3, Z6b, 1, 0)(4, $6b, 1, 0), _.I(5, "cm-icon", 2);
				}
				if (a & 2) {
					_.y(3), _.C(b.JT() ? 3 : 4), _.y(2), _.E("icon", b.icon), _.wh("aria-label", b.Tmb);
				}
			},
			dependencies: [_.SP],
			styles: [".cfc-doc-href[_nghost-%COMP%]{border-bottom:none}.cfc-doc-href-prevent-dangling-icon[_nghost-%COMP%]{white-space:nowrap;overflow-wrap:normal}.cfc-doc-href-prevent-dangling-icon[_nghost-%COMP%]   .cfc-doc-href-content[_ngcontent-%COMP%]{white-space:normal}[_nghost-%COMP%]     .cfc-doc-href-content{border-bottom:1px solid currentColor}[_nghost-%COMP%]     .cfc-icon{height:14px;margin-left:-4px}[_nghost-%COMP%]     mat-icon, [_nghost-%COMP%]    .cm-button mat-icon{margin-top:-2px;margin-right:-3px}[_nghost-%COMP%]     .cm-icon-external-link{margin-top:-2px;margin-left:-4px;height:14px;width:15px}[_nghost-%COMP%]     .cm-icon-external-link svg{margin-top:-2px}.cfc-doc-href-button[_nghost-%COMP%]   .cm-icon-external-link[_ngcontent-%COMP%]{margin-top:-4px}"]
		});
		var LQ = class {
			set Zga(a) {
				if (a) {
					a.startsWith("URL__") || a.startsWith("CONSOLE__") || a.startsWith("CLOUD_DOCS__") || a.startsWith("SUPPORT__") || (a = `CONSOLE__${a}`);
				}
				this.H.then((b) => {
					b = b.getUrl(a);
					if (new _.hk(b).ug().endsWith(".google.com")) {
						let c = y7b(this.I);
						b = Y6b(b, c === "0" ? null : c);
					}
					_.nd(this.Ga.nativeElement, b);
				});
			}
			constructor() {
				this.Dc = _.m(_.cm);
				this.Ga = _.m(_.Jf);
				this.F = _.m(_.Ui);
				this.Xc = _.m(JQ);
				this.I = _.m(vQ);
				this.H = q8b(this.Xc);
				var a = _.m(_.Xf);
				var b = _.m(_.Hg);
				var c = this.Dc.createElement("span");
				this.A = _.upa(KQ, {
					Oy: b,
					Yya: a,
					bj: c
				});
			}
			Hw() {
				return this.Ga.nativeElement.getAttribute("aria-label") || this.Ga.nativeElement.textContent.trim();
			}
			Rb() {
				_.BTa(this.F, this.A.A);
				var a = this.Ga.nativeElement.querySelector(".mat-button-wrapper, .mdc-button__label");
				if (!this.Ga.nativeElement.hasAttribute("mat-icon-button")) {
					this.A.location.nativeElement.classList.remove("cfc-doc-href"), this.A.location.nativeElement.classList.add("cfc-doc-href-button"), this.Dc.appendChild(a, this.A.location.nativeElement);
				}
				a = this.A.instance.Hw(this.Hw());
				this.Ga.nativeElement.setAttribute("aria-label", a);
			}
			Ba() {
				this.A.destroy();
			}
		};
		LQ.J = function(a) {
			return new (a || LQ)();
		};
		LQ.Oa = _.We({
			type: LQ,
			da: [
				[
					"a",
					"cfcDocHref",
					"",
					"mat-button",
					""
				],
				[
					"a",
					"cfcDocHref",
					"",
					"mat-icon-button",
					""
				],
				[
					"a",
					"cfcDocHref",
					"",
					"mat-raised-button",
					""
				],
				[
					"a",
					"cfcDocHref",
					"",
					"mat-stroked-button",
					""
				],
				[
					"a",
					"cfcDocHref",
					"",
					"mat-flat-button",
					""
				]
			],
			eb: [
				"target",
				"_blank",
				"rel",
				"noopener"
			],
			inputs: { Zga: "cfcDocHref" }
		});
		var w8b = function(a) {
			if (!a.F) {
				a.yb.runOutsideAngular(() => {
					a.F = new MutationObserver(() => {
						v8b(a);
					});
				});
			}
		};
		var v8b = function(a) {
			if (a.H && !a.H.nativeElement.children.length) {
				var b = a.H.nativeElement.childNodes;
				if (b = b[b.length - 1]) {
					a = a.trimWhitespace(b.textContent);
					if (a !== b.textContent) {
						b.textContent = a;
					}
				}
			}
		};
		var MQ = class {
			get aha() {
				return this.A;
			}
			set aha(a) {
				this.A = a;
				a = b7b(a);
				if (new _.hk(a).ug().endsWith(".google.com")) {
					let b = y7b(this.R);
					a = Y6b(a, b === "0" ? null : b);
				}
				_.nd(this.Ga.nativeElement, a);
			}
			set kFa(a) {
				if (this.H = a) {
					w8b(this);
					this.F.observe(a.nativeElement, {
						characterData: true,
						subtree: true
					});
					v8b(this);
				}
			}
			constructor() {
				this.R = _.m(vQ);
				this.Ga = _.m(_.Jf);
				this.I = _.m(_.xQ);
				this.yb = _.m(_.th);
				this.icon = _.k4b;
				this.JT = false;
				this.A = { type: 999 };
				this.Umb = "external, opens new window";
				R6b(this.Ga.nativeElement, 181355);
			}
			onClick() {
				this.I.sendEvent({
					type: "docLink",
					name: "clickLink",
					metadata: {
						type: this.A.type,
						path: this.A.path,
						suffix: this.A.suffix,
						url: this.A.url
					}
				});
			}
			Rb() {
				this.Ga.nativeElement.setAttribute("aria-label", this.Hw());
			}
			Ba() {
				if (this.F) {
					this.F.disconnect();
				}
			}
			Hw(a) {
				var b = this.Ga.nativeElement.getAttribute("aria-label");
				return `${a || b || this.Ga.nativeElement.textContent.trim()} external, opens new window`;
			}
			trimWhitespace(a) {
				return a.replace(/\s+$/, "");
			}
		};
		MQ.J = function(a) {
			return new (a || MQ)();
		};
		MQ.ka = _.u({
			type: MQ,
			da: [[
				"a",
				"cfcDocLink",
				"",
				3,
				"mat-button",
				"",
				3,
				"mat-icon-button",
				"",
				3,
				"mat-raised-button",
				"",
				3,
				"mat-stroked-button",
				"",
				3,
				"mat-flat-button",
				""
			]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ci(u8b, 5);
				}
				if (a & 2) {
					let c;
					if (_.ei(c = _.fi())) {
						b.kFa = c.first;
					}
				}
			},
			eb: [
				"target",
				"_blank",
				"rel",
				"noopener",
				1,
				"cfc-doc-link"
			],
			Ua: 2,
			Ja: function(a, b) {
				if (a & 1) {
					_.J("click", function() {
						return b.onClick();
					});
				}
				if (a & 2) {
					_.P("cfc-doc-link-prevent-dangling-icon", b.JT);
				}
			},
			inputs: {
				aha: "cfcDocLink",
				JT: [
					0,
					"cfcDocLinkPreventDanglingIcon",
					"preventDanglingIcon"
				]
			},
			fc: ["*"],
			ha: 6,
			ia: 3,
			la: [
				["ngContent", ""],
				[1, "cfc-doc-link-content"],
				[
					"role",
					"img",
					1,
					"cm-icon-external-link",
					3,
					"icon"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Xh(), _.F(0, "span", 1, 0), _.Yh(2), _.H(), _.B(3, c7b, 1, 0)(4, d7b, 1, 0), _.I(5, "cm-icon", 2);
				}
				if (a & 2) {
					_.y(3), _.C(b.JT ? 3 : 4), _.y(2), _.E("icon", b.icon), _.wh("aria-label", b.Umb);
				}
			},
			dependencies: [_.SP],
			styles: [".cfc-doc-link[_nghost-%COMP%]{border-bottom:none}.cfc-doc-link-prevent-dangling-icon[_nghost-%COMP%]{white-space:nowrap;overflow-wrap:normal}.cfc-doc-link-prevent-dangling-icon[_nghost-%COMP%]   .cfc-doc-link-content[_ngcontent-%COMP%]{white-space:normal}[_nghost-%COMP%]     .cfc-doc-link-content{border-bottom:1px solid currentColor}[_nghost-%COMP%]     .cfc-icon{height:14px;margin-left:-4px}[_nghost-%COMP%]     mat-icon, [_nghost-%COMP%]    .cm-button mat-icon{margin-top:-2px;margin-right:-3px}[_nghost-%COMP%]     .cm-icon-external-link{margin-top:-2px;margin-left:-4px;height:14px;width:15px}[_nghost-%COMP%]     .cm-icon-external-link svg{margin-top:-2px}.cfc-doc-link-button[_nghost-%COMP%]   .cm-icon-external-link[_ngcontent-%COMP%]{margin-top:-4px}.cm-gm2   .cfc-doc-link-button[_nghost-%COMP%]   .cm-icon-external-link[_ngcontent-%COMP%], .cm-cm3   .cfc-doc-link-button[_nghost-%COMP%]   .cm-icon-external-link[_ngcontent-%COMP%]{margin-top:-2px}"]
		});
		var NQ = class {
			set aha(a) {
				a = b7b(a);
				if (new _.hk(a).ug().endsWith(".google.com")) {
					let b = y7b(this.H);
					a = Y6b(a, b === "0" ? null : b);
				}
				_.nd(this.Ga.nativeElement, a);
			}
			constructor() {
				this.Dc = _.m(_.cm);
				this.Ga = _.m(_.Jf);
				this.F = _.m(_.Ui);
				this.H = _.m(vQ);
				var a = _.m(_.Hg);
				var b = _.m(_.Xf);
				var c = this.Dc.createElement("span");
				this.A = _.upa(MQ, {
					Oy: a,
					Yya: b,
					bj: c
				});
			}
			Hw() {
				return this.Ga.nativeElement.getAttribute("aria-label") || this.Ga.nativeElement.textContent.trim();
			}
			Rb() {
				_.BTa(this.F, this.A.A);
				var a = this.Ga.nativeElement.querySelector(".mat-button-wrapper, .mdc-button__label");
				if (!this.Ga.nativeElement.hasAttribute("mat-icon-button")) {
					this.A.location.nativeElement.classList.remove("cfc-doc-link"), this.A.location.nativeElement.classList.add("cfc-doc-link-button"), this.A.location.nativeElement.setAttribute("role", "img"), this.Dc.appendChild(a, this.A.location.nativeElement);
				}
				a = this.A.instance.Hw(this.Hw());
				this.Ga.nativeElement.setAttribute("aria-label", a);
			}
			Ba() {
				this.A.destroy();
			}
		};
		NQ.J = function(a) {
			return new (a || NQ)();
		};
		NQ.Oa = _.We({
			type: NQ,
			da: [
				[
					"a",
					"cfcDocLink",
					"",
					"mat-button",
					""
				],
				[
					"a",
					"cfcDocLink",
					"",
					"mat-icon-button",
					""
				],
				[
					"a",
					"cfcDocLink",
					"",
					"mat-raised-button",
					""
				],
				[
					"a",
					"cfcDocLink",
					"",
					"mat-stroked-button",
					""
				],
				[
					"a",
					"cfcDocLink",
					"",
					"mat-flat-button",
					""
				]
			],
			eb: [
				"target",
				"_blank",
				"rel",
				"noopener"
			],
			inputs: { aha: "cfcDocLink" }
		});
		var OQ = class {
			constructor() {
				this.action = _.Li.required();
				this.Qda = x8b;
				this.A = _.m(_.xQ);
			}
		};
		OQ.J = function(a) {
			return new (a || OQ)();
		};
		OQ.ka = _.u({
			type: OQ,
			da: [["cfc-tooltip-action"]],
			inputs: { action: [1, "action"] },
			ha: 1,
			ia: 1,
			la: [
				[1, "cfc-tooltip-action-button"],
				[
					"mat-button",
					"",
					"cmMatFocusIndicator",
					"",
					"color",
					"primary",
					1,
					"cm-button"
				],
				[
					"mat-button",
					"",
					"cmMatFocusIndicator",
					"",
					"color",
					"primary",
					1,
					"cm-button",
					3,
					"cfcDocLink"
				],
				[
					"mat-button",
					"",
					"cmMatFocusIndicator",
					"",
					"color",
					"primary",
					1,
					"cm-button",
					3,
					"cfcDocHref"
				],
				[
					"mat-button",
					"",
					"cmMatFocusIndicator",
					"",
					"color",
					"primary",
					1,
					"cm-button",
					3,
					"click"
				],
				[
					"mat-button",
					"",
					"cmMatFocusIndicator",
					"",
					"color",
					"primary",
					1,
					"cm-button",
					3,
					"click",
					"cfcDocLink"
				],
				[
					"mat-button",
					"",
					"cmMatFocusIndicator",
					"",
					"color",
					"primary",
					1,
					"cm-button",
					3,
					"click",
					"cfcDocHref"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, j7b, 5, 1, "div", 0);
				}
				if (a & 2) {
					let c;
					_.C((c = b.action()) ? 0 : -1, c);
				}
			},
			dependencies: [
				_.XB,
				LQ,
				NQ,
				GQ,
				_.VC,
				_.UC
			],
			styles: ["[_nghost-%COMP%]   .cfc-tooltip-action-button[_ngcontent-%COMP%], [_nghost-%COMP%]   .cfc-tooltip-action-button.cm-button[_ngcontent-%COMP%]{margin:12px 0 -12px calc(var(--cm-md1-button-padding-inline, 12px)*-1)}[_nghost-%COMP%]   .cfc-tooltip-action-button.cfc-tooltip-action-button[_ngcontent-%COMP%]{margin-bottom:-8px}[_nghost-%COMP%]   .cfc-tooltip-action-button[_ngcontent-%COMP%]   .cm-button[_ngcontent-%COMP%]{white-space:normal}"]
		});
		var y8b = (0, _.mP)`<svg data-icon-name="closeIcon" viewBox="0 0 24 24" width="24" height="24"><path fill-rule="evenodd" d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>`.firstElementChild;
		var PQ = class {
			constructor() {
				this.Hd = _.m("tooltipRef");
				this.JC = _.m("triggerElement");
				this.A = _.m(_.VA);
				this.Ga = _.m(_.Jf);
				this.Wk = { hzb: y8b };
				this.id = PQ.A++;
				this.E_ = `tooltip_overlay_${this.id}`;
				this.Wca = `tooltip_content_${this.id}`;
				this.destroy = new _.Wg();
				var a = _.m(_.Hu);
				this.Hd.zf.pipe(_.dh(this.destroy)).subscribe(() => {
					a.lb();
				});
			}
			Rb() {
				if (this.Hd.ot) {
					if (!(this.Hd.nk || this.Hd.nq !== "description")) {
						this.A.describe(this.JC, this.Ga.nativeElement, "tooltip");
					}
				} else {
					if (typeof this.Hd.content !== "string" && this.Hd.nq === "description") {
						this.A.describe(this.JC, this.Ga.nativeElement, "tooltip");
					}
				}
			}
			isString() {
				return typeof this.Hd.content === "string";
			}
			GCa() {
				return this.Hd.content instanceof _.od;
			}
			sDa(a) {
				return a instanceof _.Zh;
			}
			Ba() {
				if (this.Hd.nq === "description" || this.Hd.ot && !this.Hd.nk) {
					_.UA(this.A, this.JC, this.Ga.nativeElement, "tooltip");
				}
				this.destroy.next();
			}
			OJa() {
				return this.Hd.Tp || this.Hd.ot;
			}
		};
		PQ.A = 0;
		PQ.J = function(a) {
			return new (a || PQ)();
		};
		PQ.ka = _.u({
			type: PQ,
			da: [["cfc-tooltip-overlay"]],
			Ua: 9,
			Ja: function(a, b) {
				if (a & 2) {
					_.wh("id", b.E_)("role", b.Hd.ot ? b.Hd.nk ? "dialog" : null : typeof b.Hd.content === "string" || b.Hd.b5 ? null : "tooltip")("aria-describedby", b.Hd.nk ? b.Wca : null)("aria-label", b.Hd.nk ? b.Hd.LVa : null)("aria-modal", b.Hd.nk || null), _.P("cfc-tooltip-overlay-rich", b.Hd.d5 || b.Hd.ot)("cfc-tooltip-overlay-simple", !b.Hd.d5 && !b.Hd.ot);
				}
			},
			ha: 5,
			ia: 5,
			la: () => [
				[
					"mat-icon-button",
					"",
					"cmMatFocusIndicator",
					"",
					"aria-label",
					"Close",
					1,
					"mat-icon-button",
					"cfc-tooltip-close-button",
					"cm-button"
				],
				[
					1,
					"cfc-tooltip-content",
					3,
					"id"
				],
				[
					3,
					"ngTemplateOutlet",
					"ngTemplateOutletContext"
				],
				[
					1,
					"cm-icon--legacy-margins",
					3,
					"icon"
				],
				[
					3,
					"action",
					4,
					"ngIf"
				],
				[3, "action"],
				[3, "innerHTML"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, k7b, 2, 1, "button", 0), _.F(1, "div", 1), _.B(2, m7b, 2, 2), _.B(3, o7b, 2, 2), _.B(4, p7b, 1, 2, "div", 2), _.H();
				}
				if (a & 2) {
					_.C(b.OJa() ? 0 : -1), _.y(), _.E("id", b.Wca), _.y(), _.C(b.isString() ? 2 : -1), _.y(), _.C(b.GCa() ? 3 : -1), _.y(), _.C(b.sDa(b.Hd.content) ? 4 : -1);
				}
			},
			dependencies: [
				_.YB,
				_.SP,
				_.mz,
				_.nz,
				OQ,
				GQ,
				_.VC,
				_.UC
			],
			styles: ["cfc-tooltip-overlay[_nghost-%COMP%]{color:var(--cm-sys-color-on-surface,#000);font:var(--cm-sys-type-body-small,400 12px/16px \"Roboto\",sans-serif);display:flex;flex-direction:row-reverse;border-radius:4px;margin:8px;word-wrap:break-word;outline:1px solid transparent;--cm-comp-button-non-primary-default-label-color:var(--cm-sys-color-on-surface-variant,rgba(0,0,0,.66))}cfc-tooltip-overlay.cfc-tooltip-overlay-simple[_nghost-%COMP%]{max-width:320px;padding:4px 8px;width:max-content;color:var(--cm-sys-color-on-surface-inverse,#fff);background:light-dark(#3c4043,#e8eaed)}cfc-tooltip-overlay.cfc-tooltip-overlay-simple[_nghost-%COMP%]   .cfc-tooltip-close-button[_ngcontent-%COMP%]{color:var(--cm-sys-color-on-primary,#fff)}cfc-tooltip-overlay.cfc-tooltip-overlay-rich[_nghost-%COMP%]{max-height:384px;max-width:384px;overflow-y:auto;padding:16px}cfc-tooltip-overlay[_nghost-%COMP%]   .cfc-tooltip-close-button[_ngcontent-%COMP%]{margin:-12px;margin-bottom:-11.9px;margin-left:8px}cfc-tooltip-overlay[_nghost-%COMP%]   .cfc-tooltip-content[_ngcontent-%COMP%]{overflow-wrap:anywhere}@media screen and (hover:none){cfc-tooltip-overlay[_nghost-%COMP%]   .cfc-tooltip-disable-user-select-on-touch-device[_ngcontent-%COMP%]{-webkit-touch-callout:none;user-select:none;-webkit-user-select:none}}cfc-tooltip-overlay.cfc-tooltip-overlay-rich[_nghost-%COMP%]{box-shadow:var(--cm-sys-elevation-shadow,0 1px 8px 0 rgba(0,0,0,.2),0 3px 4px 0 rgba(0,0,0,.14),0 3px 3px -2px rgba(0,0,0,.12));color:var(--cm-sys-color-on-surface,#000);background:var(--cm-sys-color-surface-elevation,#fff)}"]
		});
		var QQ = [
			{
				Lb: "start",
				Mb: "center",
				Bb: "end",
				Gb: "center"
			},
			{
				Lb: "start",
				Mb: "top",
				Bb: "end",
				Gb: "top"
			},
			{
				Lb: "start",
				Mb: "bottom",
				Bb: "end",
				Gb: "bottom"
			}
		];
		var RQ = [
			{
				Lb: "end",
				Mb: "center",
				Bb: "start",
				Gb: "center"
			},
			{
				Lb: "end",
				Mb: "top",
				Bb: "start",
				Gb: "top"
			},
			{
				Lb: "end",
				Mb: "bottom",
				Bb: "start",
				Gb: "bottom"
			}
		];
		var SQ = [
			{
				Lb: "center",
				Mb: "top",
				Bb: "center",
				Gb: "bottom"
			},
			{
				Lb: "start",
				Mb: "top",
				Bb: "start",
				Gb: "bottom"
			},
			{
				Lb: "end",
				Mb: "top",
				Bb: "end",
				Gb: "bottom"
			}
		];
		var z8b = [
			{
				Lb: "start",
				Mb: "top",
				Bb: "start",
				Gb: "bottom"
			},
			{
				Lb: "center",
				Mb: "top",
				Bb: "center",
				Gb: "bottom"
			},
			{
				Lb: "end",
				Mb: "top",
				Bb: "end",
				Gb: "bottom"
			}
		];
		var TQ = [
			{
				Lb: "center",
				Mb: "bottom",
				Bb: "center",
				Gb: "top"
			},
			{
				Lb: "start",
				Mb: "bottom",
				Bb: "start",
				Gb: "top"
			},
			{
				Lb: "end",
				Mb: "bottom",
				Bb: "end",
				Gb: "top"
			}
		];
		var A8b = [
			{
				Lb: "start",
				Mb: "bottom",
				Bb: "start",
				Gb: "top"
			},
			{
				Lb: "center",
				Mb: "bottom",
				Bb: "center",
				Gb: "top"
			},
			{
				Lb: "end",
				Mb: "bottom",
				Bb: "end",
				Gb: "top"
			}
		];
		var B8b = [
			{
				Lb: "end",
				Mb: "center",
				Bb: "start",
				Gb: "top"
			},
			{
				Lb: "end",
				Mb: "center",
				Bb: "start",
				Gb: "center"
			},
			{
				Lb: "end",
				Mb: "center",
				Bb: "start",
				Gb: "bottom"
			}
		];
		var C8b = {
			top: [
				...SQ,
				...TQ,
				...QQ,
				...RQ
			],
			topleft: [
				...z8b,
				...A8b,
				...QQ,
				...RQ
			],
			bottom: [
				...TQ,
				...SQ,
				...RQ,
				...QQ
			],
			bottomleft: [
				...A8b,
				...z8b,
				...RQ,
				...QQ
			],
			left: [
				...QQ,
				...RQ,
				...SQ,
				...TQ
			],
			right: [
				...RQ,
				...QQ,
				...TQ,
				...SQ
			],
			righttop: [
				{
					Lb: "end",
					Mb: "center",
					Bb: "start",
					Gb: "bottom"
				},
				{
					Lb: "end",
					Mb: "center",
					Bb: "start",
					Gb: "center"
				},
				{
					Lb: "end",
					Mb: "top",
					Bb: "start",
					Gb: "top"
				},
				...QQ,
				...TQ,
				...SQ
			],
			lefttop: [
				{
					Lb: "start",
					Mb: "center",
					Bb: "end",
					Gb: "bottom"
				},
				{
					Lb: "start",
					Mb: "center",
					Bb: "end",
					Gb: "center"
				},
				{
					Lb: "start",
					Mb: "top",
					Bb: "end",
					Gb: "top"
				},
				...QQ,
				...TQ,
				...RQ
			],
			leftbottom: [
				{
					Lb: "start",
					Mb: "center",
					Bb: "end",
					Gb: "top"
				},
				{
					Lb: "start",
					Mb: "center",
					Bb: "end",
					Gb: "center"
				},
				{
					Lb: "start",
					Mb: "center",
					Bb: "start",
					Gb: "bottom"
				},
				...TQ,
				...B8b,
				...SQ
			],
			rightbottom: [
				...B8b,
				...TQ,
				...QQ,
				...SQ
			]
		};
		var D8b = function(a, b, c) {
			c = _.uib(_.qB(_.rB(_.vB(a.overlay.position(), b), false), c), true);
			_.sib(c, _.aib(a.A, b));
			b = new _.dm();
			b.yg = c;
			b.zj = a.overlay.A.A();
			return b;
		};
		var UQ = class {
			constructor() {
				this.overlay = _.m(_.EB);
				this.A = _.m(_.Tl);
			}
		};
		UQ.J = function(a) {
			return new (a || UQ)();
		};
		UQ.sa = _.Cd({
			token: UQ,
			factory: UQ.J,
			wa: "root"
		});
		var VQ = class {
			constructor() {
				this.overlay = _.m(_.EB);
				this.xxa = _.m(UQ);
				this.U3 = _.m(FQ);
				this.h7 = _.m(_.XA);
				this.Mj = _.m(_.SA);
				this.yb = _.m(_.th);
				this.qh = undefined;
				this.tooltips = new Map();
				this.Nb = undefined;
				this.TF = this.wJ = 0;
				this.Sh = undefined;
				this.HS = 0;
				this.Hc = new _.Wg();
				this.backdrop = undefined;
				this.R6 = "mouseover mouseout focusin focusout touchstart touchend touchcancel click keydown".split(" ");
				var a = this.yb;
				document.addEventListener("click", this);
				a.runOutsideAngular(() => {
					for (let b of this.R6) document.addEventListener(b, this);
				});
			}
			VHa(a) {
				var b = a.Ga.nativeElement;
				var c = this.tooltips.get(b) || {};
				c.sH = a;
				this.tooltips.set(b, c);
			}
			x9(a) {
				return !!this.Nb && this.Sh === a;
			}
			UHa(a, b) {
				var c = a.Ga.nativeElement;
				var d = this.tooltips.get(c) || {};
				d.Qf = a;
				d.x6 = b.subscribe((e) => {
					if (!(e || this.Sh !== a)) {
						this.El();
					}
				});
				this.tooltips.set(c, d);
			}
			q6(a) {
				this.p6(a, false);
			}
			Zha(a) {
				this.p6(a, true);
			}
			Ba() {
				for (let a of this.tooltips.values()) a.Qf && a.x6.unsubscribe();
				for (let a of this.R6) document.removeEventListener(a, this);
				this.BQ();
				this.Hc.next();
				this.Hc.complete();
			}
			p6(a, b) {
				if (this.Sh === a) {
					this.El();
				}
				if (this.mN === a) {
					clearTimeout(this.TF), clearTimeout(this.gV), this.mN = undefined;
				}
				var c;
				a = (c = a.Ga) == null ? undefined : c.nativeElement;
				if (a != null && this.tooltips.has(a)) {
					c = this.tooltips.get(a), b ? (c.x6.unsubscribe(), c.Qf = undefined) : c.sH = undefined, c.Qf || c.sH ? this.tooltips.set(a, c) : this.tooltips.delete(a);
				}
			}
			tAa(a) {
				if (a = _.Mv(a.target, (c) => {
					if (!c) return false;
					var d = this.yc(c);
					return this.tooltips.has(c) ? (c = this.tooltips.get(c), !d && c.sH !== undefined || d && c.Qf !== undefined) : false;
				})) {
					var b = this.tooltips.get(a);
					return this.yc(a) ? b.Qf : b.sH;
				}
			}
			yc(a) {
				return a.disabled || a instanceof Element && (a.getAttribute("aria-disabled") === "true" || a.classList.contains("cfc-disabled"));
			}
			handleEvent(a) {
				if (this.Sh && this.Sh.nk && a.type === "click" && a.target.classList.contains("cdk-overlay-backdrop")) this.yb.run(() => {
					this.El();
				});
				else if (this.U8(a.target)) this.yb.run(() => {
					this.PAa(a);
				});
				else {
					var b = this.tAa(a);
					if (b) {
						this.yb.run(() => {
							this.RAa(a, b);
						});
					} else {
						if (this.Sh && a.type === "click" && !this.Sh.nk) {
							this.yb.run(() => {
								this.El();
							});
						}
					}
				}
			}
			RAa(a, b) {
				if (a.type === "mouseout") {
					this.NAa(b);
				} else {
					a.type === "mouseover" ? this.Iv(b, a, true) : a.type === "focusin" ? b.Fwa ? this.Iv(b, a, true) : b.ot || document.body.classList.contains("cfc-keyboard-modality") && this.Iv(b, a, true) : a.type === "focusout" ? this.LAa(a) : a.type === "touchstart" ? this.TAa(a, b) : a.type === "touchend" || a.type === "touchcancel" ? this.SAa() : (a.type === "click" || a.type === "keydown" && a.key === "Enter") && this.KAa(b, a);
				}
			}
			KAa(a, b) {
				if (a.ot && this.S4(a)) {
					this.Sh || this.Iv(a, b, false), this.tEa();
				}
			}
			tEa() {
				var a = this.Sh;
				if (this.Nb) {
					var b = this.Nb;
					if (!a.nk) {
						a.nk = true, a.zf.next(), h8b(this.U3, b), this.PBa(), this.BQ(), this.qh = this.h7.create(b.bj), setTimeout(() => {
							var c;
							if (!((c = this.qh) == null)) {
								_.n4b(c);
							}
						});
					}
				}
			}
			NAa(a) {
				if (!a.nk) {
					a.d5 || a.ot || a.MVa ? this.ica() : this.El();
				}
			}
			TAa(a, b) {
				this.HS = Date.now();
				clearTimeout(this.gV);
				this.mN = b;
				this.gV = setTimeout(() => {
					this.Iv(b, a, false);
					this.mN = undefined;
				}, 251);
			}
			f9() {
				return Date.now() - this.HS < 1e3;
			}
			SAa() {
				this.HS = Date.now();
				clearTimeout(this.gV);
				this.mN = undefined;
			}
			U8(a) {
				return !!this.Nb && _.TVa(this.Nb.bj, a);
			}
			LAa(a) {
				if (!this.U8(a.relatedTarget)) {
					this.El();
				}
			}
			PAa(a) {
				if (a.type === "mouseover") {
					clearTimeout(this.wJ);
				} else {
					a.type !== "mouseout" || this.Sh && this.Sh.nk ? (a.type === "click" || a.type === "keydown" && a.key === "Enter") && this.QAa(a) : this.ica();
				}
			}
			QAa(a) {
				if (_.o4b(a.target, "cfc-tooltip-close-button")) {
					a.preventDefault(), a.stopPropagation(), this.El();
				}
			}
			ica() {
				clearTimeout(this.wJ);
				this.wJ = setTimeout(() => {
					this.El();
				}, 100);
			}
			TKa(a) {
				return a.bha !== undefined ? a.bha : a.ot || a.Ga.nativeElement.classList.contains("cfc-icon") || a.Ga.nativeElement.tagName === "CM-ICON" ? true : false;
			}
			S4(a) {
				return !a.b5 && !!a.content;
			}
			Iv(a, b, c) {
				if (!(!this.S4(a) || b.type !== "touchstart" && this.f9() && !this.TKa(a))) {
					a.Tp = this.f9() || b.type === "touchstart", clearTimeout(this.wJ), this.Nb && this.Sh === a || (this.Nb && this.El(), clearTimeout(this.TF), clearTimeout(this.gV), this.mN = undefined, c && a.Hwa > 0 ? (this.mN = a, this.TF = setTimeout(() => {
						this.Oaa(a, b);
						this.mN = undefined;
					}, a.Hwa)) : this.Oaa(a, b));
				}
			}
			Oaa(a, b) {
				if (a.Ga.nativeElement.isConnected) {
					a.eC.next();
					var c = D8b(this.xxa, a.Ga, C8b[a.c5]);
					this.Nb = this.overlay.create(c);
					b = _.Xi({ vd: [{
						Da: "tooltipRef",
						Vc: a
					}, {
						Da: "triggerElement",
						Vc: b.target
					}] });
					this.Nb.attach(new _.xB(PQ, undefined, b));
					this.Nb.kx().pipe(_.dh(this.Hc)).subscribe((d) => {
						if (d.key === "Escape") {
							this.El();
						}
					});
					this.Sh = a;
				}
			}
			PBa() {
				if (this.Nb) {
					var a = document.createElement("div");
					a.classList.add("cdk-overlay-backdrop");
					a.classList.add("cdk-overlay-dark-backdrop");
					a.classList.add("cdk-overlay-backdrop-showing");
					var b = this.Nb.bj;
					b.insertBefore(a, b.firstChild);
					this.backdrop = a;
				}
			}
			dIa() {
				if (this.backdrop && this.backdrop.parentNode) {
					this.backdrop.parentNode.removeChild(this.backdrop), this.backdrop = undefined;
				}
			}
			El(a) {
				if (!a || a === this.Sh) {
					if (this.TF && (clearTimeout(this.TF), this.mN = undefined), this.Nb) {
						if ((a = this.Sh) && a.nk) {
							this.dIa();
							a.nk = false;
							a.zf.next();
							var b = this.U3;
							let c = b.A[b.A.length - 1];
							if (c && this.Nb === c.Nb) {
								do
									i8b(b.A.pop()), c = b.A[b.A.length - 1];
								while (c && !c.Nb.Ug());
							}
							this.BQ();
							if (a.i7) {
								a.i7.nativeElement.focus();
							} else {
								a.Ga.nativeElement.focus();
							}
						}
						if (a) {
							a.ZB.next();
						}
						if (this.Nb) {
							this.Nb.detach(), this.Nb.dispose();
						}
						this.Sh = this.Nb = undefined;
					}
				}
			}
			BQ() {
				if (this.qh) {
					this.qh.destroy(), this.qh = undefined;
				}
			}
		};
		VQ.J = function(a) {
			return new (a || VQ)();
		};
		VQ.sa = _.Cd({
			token: VQ,
			factory: VQ.J,
			wa: "root"
		});
		var F8b = _.ZP("google.internal.cloud.usersettings.settings.pancore.a11y.KeyboardShortcutAction", "[null,[[\"ACTION_UNSPECIFIED\",0],[\"ACTION_CLOSE_DRAWER\",1],[\"ACTION_GO_UP_ONE_PAGE\",2],[\"ACTION_TOGGLE_DEBUG_LOG_WINDOW\",3],[\"ACTION_SNACKBAR_FOCUS\",17],[\"ACTION_SNACKBAR_CLOSE\",47],[\"ACTION_DEMO_PAGE\",15],[\"ACTION_TEST\",16],[\"ACTION_TEST_SEQUENTIAL\",29],[\"ACTION_OPEN_CLOUD_SHELL\",4],[\"ACTION_OPEN_CONSOLE_NAV_PANEL\",5],[\"ACTION_OPEN_FEEDBACK_WIDGET\",6],[\"ACTION_OPEN_HELP_MENU\",7],[\"ACTION_OPEN_PURVIEW_SWITCHER\",8],[\"ACTION_OPEN_SEARCH_BAR\",9],[\"ACTION_OPEN_SHORTCUT_DIALOG\",10],[\"ACTION_OPEN_UTILITIES_MENU\",54],[\"ACTION_OPEN_NOTIFICATIONS_PANEL\",53],[\"ACTION_TOGGLE_BUILDER_COMMAND\",13],[\"ACTION_TOGGLE_LIBRARY_COMMAND\",14],[\"ACTION_NAVIGATE_PREVIOUS_LOG\",55],[\"ACTION_NAVIGATE_NEXT_LOG\",56],[\"ACTION_BIGQUERY_MOVE_TAB_LEFT\",18],[\"ACTION_BIGQUERY_MOVE_TAB_RIGHT\",19],[\"ACTION_BIGQUERY_TOGGLE_LINE_COMMENT\",30],[\"ACTION_BIGQUERY_JUMP_NEXT_TAB\",34],[\"ACTION_BIGQUERY_JUMP_PREVIOUS_TAB\",35],[\"ACTION_BIGQUERY_SPLIT_TAB_RIGHT\",36],[\"ACTION_BIGQUERY_SPLIT_TAB_LEFT\",37],[\"ACTION_BIGQUERY_JUMP_TO_TAB_1\",38],[\"ACTION_BIGQUERY_JUMP_TO_TAB_2\",39],[\"ACTION_BIGQUERY_JUMP_TO_TAB_3\",40],[\"ACTION_BIGQUERY_JUMP_TO_TAB_4\",41],[\"ACTION_BIGQUERY_JUMP_TO_TAB_5\",42],[\"ACTION_BIGQUERY_JUMP_TO_TAB_6\",43],[\"ACTION_BIGQUERY_JUMP_TO_TAB_7\",44],[\"ACTION_BIGQUERY_JUMP_TO_TAB_8\",45],[\"ACTION_BIGQUERY_JUMP_TO_TAB_9\",46],[\"ACTION_BIGQUERY_CREATE_NEW_TAB\",48],[\"ACTION_BIGTABLE_RUN_QUERY\",31],[\"ACTION_EXPAND_QUERY_EDITOR\",28],[\"ACTION_OPEN_FIND_BAR\",20],[\"ACTION_FIND_NEXT\",21],[\"ACTION_FIND_PREV\",22],[\"ACTION_TOGGLE_REGEX\",23],[\"ACTION_TOGGLE_CASE_SENSITIVE\",24],[\"ACTION_CANCEL\",25],[\"ACTION_NEXT_ITEM\",26],[\"ACTION_PREV_ITEM\",27],[\"ACTION_OPEN_SUMMARY_SUBTASK\",32],[\"ACTION_OPEN_GEMINI\",49],[\"ACTION_IAM_OPEN_ROLES_SUGGESTION\",51],[\"ACTION_IAM_ADD_ROLE\",52]],null,[[11,11],[12,12],[33,33],[50,50]]]");
		G8b.get = _.YP(class extends _.h {
			constructor(a) {
				super(a);
			}
			getKey(a) {
				return _.at(this, 1, a);
			}
		}, "google.internal.cloud.usersettings.settings.pancore.a11y.KeyboardShortcutEvent", 2, "[null,[[\"key\",null,1,3,9],[\"alt_key\",null,2,1,8],[\"ctrl_key\",null,3,1,8],[\"meta_key\",null,4,1,8],[\"shift_key\",null,5,1,8]]]");
		var H8b = {};
		H8b.get = _.YP(class extends _.h {
			constructor(a) {
				super(a);
			}
			getAction() {
				return _.Lm(this, 2);
			}
		}, "google.internal.cloud.usersettings.settings.pancore.a11y.KeyboardShortcut", 2, "[null,[[\"action\",null,2,1,14,\".google.internal.cloud.usersettings.settings.pancore.a11y.KeyboardShortcutAction\"],[\"event\",null,1,1,11,\".google.internal.cloud.usersettings.settings.pancore.a11y.KeyboardShortcutEvent\"],[\"enabled\",null,3,1,8]]]", G8b.get, F8b);
		var I8b = _.Q4b(F8b());
		_.O4b(F8b());
		H8b.get();
		var WQ = class {
			constructor() {
				this.F = _.m(yQ);
				this.A = this.F.create("globalShortcutHintsCacheId", "Globally-shared mapping of shortcut keys to shortcut hints.");
			}
			register(a, b) {
				b = b !== undefined ? I8b.get(b) : undefined;
				var c = b || a.keys.toString();
				if (c.length === 0) throw Error("fg");
				if (this.A.has(c)) {
					_.rP().logClientError({
						message: "Shortcut hint already registered.",
						eventName: "shortcutHintAlreadyRegistered",
						metadata: { shortcutHint: JSON.stringify(a) }
					});
				}
				this.A.set(c, a);
				return () => {
					this.A.delete(c);
				};
			}
		};
		WQ.J = function(a) {
			return new (a || WQ)();
		};
		WQ.sa = _.Cd({
			token: WQ,
			factory: WQ.J,
			wa: "root"
		});
		new _.he("");
		_.W(() => false);
		_.W(() => []);
		_.W(() => {
			throw new _.Sd(1905, false);
		});
		_.W(() => []);
		_.W(() => false);
		new _.he("");
		var J8b = {
			disabled: "disabled",
			w6b: "disabledReasons",
			dirty: "dirty",
			errors: "errors",
			hidden: "hidden",
			invalid: "invalid",
			max: "max",
			maxLength: "maxLength",
			min: "min",
			minLength: "minLength",
			name: "name",
			pattern: "pattern",
			pending: "pending",
			readonly: "readonly",
			required: "required",
			touched: "touched"
		};
		for (let a of Object.keys(J8b));
		Object.values(J8b);
		var K8b = new _.he("");
		var L8b = function(a) {
			if (a.Nl) return a.Nl && !a.Nl.ot && typeof a.Nl.jw === "string" ? a.Nl.jw : "Press enter to go to the tooltip.";
		};
		var M8b = function(a) {
			if (a.A.Ga && (a.Nl !== undefined || a.F.jw || a.A.jw)) {
				a.ta ? a.Nl !== a.F && (a.Nl && a.XJ.Zha(a.Nl), a.XJ.UHa(a.F, a.Z4), a.Ewa.emit(a.F), a.Nl = a.F) : a.yc && a.Nl !== a.A && (a.Nl && a.XJ.Zha(a.Nl), a.XJ.UHa(a.A, a.Z4), a.Ewa.emit(a.A), a.Nl = a.A);
			}
		};
		var P8b = function(a, b) {
			if (a.Nl) {
				var c = a.eE();
				var d = L8b(a);
				if (b) {
					a.Nl && a.Nl.nq === "label" ? X7b(c.nativeElement, b, N8b(b)) : _.UA(a.na, c.nativeElement, b, N8b(b));
				}
				if (a.yc) {
					d && (a.Nl && a.Nl.nq === "label" ? U7b(a.za, c.nativeElement, d, N8b(d)) : a.na.describe(c.nativeElement, d, N8b(d))), O8b(a);
				}
			}
		};
		var O8b = function(a) {
			var b = a.eE();
			b.nativeElement.addEventListener("focus", () => {
				if (a.yc) {
					a.fa = a.Fa.register(E8b);
				}
			});
			b.nativeElement.addEventListener("blur", () => {
				a.fa();
			});
		};
		var N8b = function(a) {
			return a === "Press enter to go to the tooltip." ? undefined : "tooltip";
		};
		var XQ = class {
			set disabled(a) {
				if (a !== "") {
					this.FQ = _.Ml(a);
				}
				this.H();
			}
			get yc() {
				return this.FQ || this.R || this.ta;
			}
			set Dyb(a) {
				var b = L8b(this);
				this.A.jw = a || "";
				this.H(b);
			}
			set Hyb(a) {
				this.A.cQ = a;
				this.F.cQ = a;
			}
			set Fyb(a) {
				this.A.c5 = a || "bottomleft";
				this.F.c5 = a || "bottomleft";
			}
			set Gyb(a) {
				this.A.ot = a;
			}
			set Eyb(a) {
				this.A.a5 = a;
			}
			wh() {}
			constructor(a, b, c, d, e, f, g) {
				this.za = a;
				this.na = b;
				this.ma = c;
				this.XJ = d;
				this.Ga = e;
				this.Fa = f;
				this.control = g;
				this.ea = _.m(K8b, {
					optional: true,
					host: true
				});
				this.Pa = _.m(_.Xf);
				this.R = this.FQ = false;
				this.Z4 = new _.pm();
				this.Ewa = new _.pm();
				this.fa = () => {};
				this.oa = this.destroyed = this.aa = false;
				this.A = new d8b();
				this.F = new d8b();
				this.Hc = new _.Wg();
				this.Ea = [];
			}
			Rb() {
				this.A.Ga = this.UY();
				this.A.i7 = this.eE();
				this.F.Ga = this.UY();
				this.F.i7 = this.eE();
				this.ma.A.add(this.Ga.nativeElement);
				if (this.ea) {
					this.R = this.ea.state().disabled();
					_.Kg(() => {
						this.R = this.ea.state().disabled();
						this.U();
						this.H();
					}, { Pa: this.Pa });
				} else {
					var a;
					if ((a = this.control) == null ? 0 : a.control) {
						a = this.control.control;
						this.R = a.disabled;
						_.dza(a, (b) => {
							if (!(this.destroyed || b === this.R)) {
								this.R = b, this.U(), this.H();
							}
						});
					}
				}
				this.oa = true;
				this.H();
			}
			fT() {
				this.X(this.yc);
			}
			Wb() {
				this.U();
			}
			U() {
				Object.keys(this.A.cQ);
			}
			Ba() {
				this.Hc.next();
				this.fa();
				this.destroyed = true;
				if (this.Nl) {
					this.XJ.Zha(this.Nl);
				}
				this.ma.A.delete(this.Ga.nativeElement);
			}
			X(a) {
				var b = this.UY().nativeElement;
				if (b) {
					b.classList.toggle("cfc-disabled", a);
				}
				if (b = this.eE().nativeElement) {
					a ? b.setAttribute("aria-disabled", "true") : b.removeAttribute("aria-disabled");
				}
			}
			eE() {
				return this.Ga;
			}
			UY() {
				return this.Ga;
			}
			H(a) {
				if (this.oa) {
					this.X(this.yc);
					if (this.yc !== this.aa) {
						this.aa = this.yc, this.Z4.emit(this.yc);
					}
					var b = L8b(this);
					M8b(this);
					P8b(this, a || b);
				}
			}
			get ta() {
				return this.Ea.length > 0 && this.Aa && this.Aa.disabled;
			}
		};
		XQ.J = function(a) {
			return new (a || XQ)(_.Dg(EQ), _.Dg(_.VA), _.Dg(zQ), _.Dg(VQ), _.Dg(_.Jf), _.Dg(WQ), _.Dg(_.lD), _.Dg(_.Hu));
		};
		XQ.Oa = _.We({
			type: XQ,
			Ja: function(a, b) {
				if (a & 1) {
					_.J("keydown.escape", function(c) {
						if (b.yc && b.Nl && b.XJ.x9(b.Nl)) {
							c.stopPropagation(), c.preventDefault(), b.XJ.El(b.Nl), _.RA(b.XJ.Mj, "Tooltip dismissed");
						}
					})("blur", function() {
						return b.wh();
					});
				}
			},
			inputs: {
				disabled: [
					0,
					"cfcDisable",
					"disabled"
				],
				Dyb: "cfcDisableTooltip",
				Hyb: "cfcDisableTooltipTemplateData",
				Fyb: "cfcDisableTooltipPosition",
				Gyb: "cfcDisableTooltipRich",
				Eyb: "cfcDisableTooltipAction"
			},
			outputs: {
				Z4: "cfcDisabledChange",
				Ewa: "cfcDisabledTooltipChange"
			},
			standalone: false,
			features: [_.su]
		});
		var Q8b = new _.he("DisablePluginHost");
		var YQ = class extends XQ {
			set disabled(a) {
				this.FQ = _.Ml(a);
				this.H();
			}
			constructor() {
				var a = _.m(EQ);
				var b = _.m(_.VA);
				var c = _.m(zQ);
				var d = _.m(VQ);
				var e = _.m(_.Jf);
				var f = _.m(WQ);
				var g = _.m(_.lD, {
					host: true,
					optional: true
				});
				var k = _.m(_.Hu);
				super(a, b, c, d, e, f, g, k);
				this.I = _.m(R7b);
			}
			Rb() {
				var a;
				var b;
				if ((b = (a = this.I).D9b) == null || !b.call(a) || this.I.eE() && this.I.eE().nativeElement) {
					super.Rb();
				} else {
					_.bg(() => {
						super.Rb();
					}, { Pa: this.Pa });
				}
			}
			Wb(a) {
				this.U();
				if (a.disabled) {
					this.I.zJa(this.yc, !!this.A.jw || !!this.F.jw);
				}
			}
			U() {
				Object.keys(this.A.cQ);
			}
			X(a) {
				this.I.zJa(a, !!this.A.jw || !!this.F.jw);
			}
			H(a) {
				this.X(this.yc);
				if (this.yc !== this.aa) {
					this.aa = this.yc, this.Z4.emit(this.yc);
				}
				var b = L8b(this);
				M8b(this);
				P8b(this, a || b);
			}
			eE() {
				return this.I.eE();
			}
			UY() {
				return this.I.UY();
			}
		};
		YQ.J = function(a) {
			return new (a || YQ)();
		};
		YQ.Oa = _.We({
			type: YQ,
			inputs: { disabled: "disabled" },
			features: [
				_.yi([{
					Da: Q8b,
					zb: YQ
				}, {
					Da: XQ,
					zb: YQ
				}]),
				_.nh,
				_.su
			]
		});
		_.ZQ = class extends AQ {
			constructor(a, b) {
				super(a, b);
				this.I = false;
				_.m(O7b, { optional: true });
				this.H = _.m(_.P7b, { optional: true });
				this.R = _.m(_.BQ);
				this.ub = _.m(_.ag);
				this.F = _.m(_.XB, { optional: true });
				this.U = _.m(_.YB, { optional: true });
				this.X = _.m(uQ, { optional: true });
				this.theme = this.R.X.pipe(_.uf((d) => d)).pipe(_.Ak(this.ub));
				var c;
				if (this.F && ((c = this.H) == null ? 0 : c.wQ)) {
					let d;
					_.WB(this.F, (d = this.H) == null ? undefined : d.wQ);
				}
			}
			Rb() {
				super.Rb();
				var a = this.Ga.nativeElement;
				a.classList.contains("mat-raised-button");
				var b = a.hasAttribute("mat-flat-button");
				this.theme.subscribe({ next: (c) => {
					c = c === "cm3";
					if (a.hasAttribute("mat-raised-button")) {
						if (a.classList.contains("mat-primary")) {
							let f;
							if (!((f = this.F) == null)) {
								_.WB(f, "filled");
							}
						} else {
							let f;
							if (!((f = this.F) == null)) {
								_.WB(f, "outlined");
							}
							a.classList.add("mat-primary");
						}
						a.classList.remove("mdc-button--raised");
						a.classList.remove("mat-mdc-raised-button");
					}
					var d;
					if (c && ((d = this.F) == null ? undefined : d.appearance) === "text") {
						_.WB(this.F, "outlined");
					}
					var e;
					if ((e = this.H) == null ? 0 : e.sMa) if (this.H.sMa === "message") {
						if (!b && !a.hasAttribute("mat-icon-button")) {
							a.classList.remove("mat-primary");
							a.classList.add("mat-unthemed");
							let f;
							if (!((f = this.F) == null)) {
								_.WB(f, "outlined");
							}
						}
					} else switch (this.H.sMa) {
						case "basic":
							let f;
							(f = this.F) == null || _.WB(f, "text");
							break;
						case "stroked":
							let g;
							(g = this.F) == null || _.WB(g, "outlined");
							break;
						case "flat":
							let k;
							(k = this.F) == null || _.WB(k, "filled");
					}
				} });
			}
			zJa(a, b) {
				if (this.I = this.I || b) {
					this.button.disabled = a;
					this.button.sd = b;
				}
			}
			Mga() {
				return this.disabled || this.button.disabled;
			}
			get button() {
				return this.F || this.U || this.X;
			}
		};
		_.ZQ.J = function(a) {
			return new (a || _.ZQ)(_.Dg(_.Jf), _.Dg(zQ));
		};
		_.ZQ.Oa = _.We({
			type: _.ZQ,
			da: [
				[
					"button",
					"mat-button",
					""
				],
				[
					"a",
					"mat-button",
					""
				],
				[
					"",
					"matButton",
					"text"
				],
				[
					"",
					"matButton",
					""
				],
				[
					"button",
					"mat-mini-fab",
					""
				],
				[
					"button",
					"matMiniFab",
					""
				],
				[
					"button",
					"mat-raised-button",
					""
				],
				[
					"a",
					"mat-raised-button",
					""
				],
				[
					"button",
					"mat-icon-button",
					""
				],
				[
					"a",
					"mat-icon-button",
					""
				],
				["matIconButton"],
				[
					"button",
					"mat-flat-button",
					""
				],
				[
					"a",
					"mat-flat-button",
					""
				],
				[
					"",
					"matButton",
					"filled"
				],
				[
					"button",
					"mat-stroked-button",
					""
				],
				[
					"a",
					"mat-stroked-button",
					""
				],
				[
					"",
					"matButton",
					"outlined"
				]
			],
			Ua: 11,
			Ja: function(a, b) {
				if (a & 2) {
					_.wh("disabled", b.button.sd || !b.button.disabled ? null : true)("aria-disabled", b.Mga() ? true : null)("mat-ripple-loader-disabled", b.Mga() ? true : null), _.P("cm-button", true)("cm-disabled", b.Mga())("mat-mdc-button-disabled", b.Mga())("mat-mdc-button-disabled-interactive", b.disabled || b.button.sd);
				}
			},
			Cc: ["cmMatButton"],
			standalone: false,
			features: [
				_.yi([{
					Da: _.gjb,
					ke: () => {
						var a = _.m(_.P7b, { optional: true });
						return a ? {
							color: a == null ? undefined : a.color,
							sd: a == null ? undefined : a.sd,
							wQ: a == null ? undefined : a.wQ
						} : null;
					}
				}, {
					Da: R7b,
					zb: _.ZQ
				}]),
				_.mh([{
					directive: YQ,
					inputs: "disabled disabled cfcDisableTooltip disabledTooltip cfcDisableTooltipTemplateData disabledTooltipTemplateData cfcDisableTooltipPosition disabledTooltipPosition cfcDisableTooltipRich disabledTooltipRich cfcDisableTooltipAction disabledTooltipAction".split(" "),
					outputs: [
						"cfcDisabledChange",
						"disabledChange",
						"cfcDisabledTooltipChange",
						"disabledTooltipChange"
					]
				}]),
				_.nh
			]
		});
		_.$Q = class {};
		_.$Q.J = function(a) {
			return new (a || _.$Q)();
		};
		_.$Q.qc = _.Ve({ type: _.$Q });
		_.$Q.oc = _.Dd({ imports: [_.VC, _.VC] });
		var T8b;
		var R8b;
		var S8b;
		T8b = function(a, b) {
			var c = typeof b === "string" || a.nq === "description" || a.nq === "label";
			if (a.I && c) {
				c = () => {
					e = R8b(a);
				};
				var d = () => {
					if (typeof b === "string") {
						_.UA(a.F, e, b, "tooltip"), X7b(e, b, "tooltip");
					}
					if (a.nq === "description") {
						var f = e;
						if (!(a.ot || typeof a.content !== "string")) {
							S8b(a) ? _.UA(a.F, f, a.content, "tooltip") : a.F.describe(f, a.content, "tooltip");
						}
					} else a.nq === "label" && (f = e, typeof a.content === "string" && (S8b(a) ? X7b(f, a.content, "tooltip") : U7b(a.U, f, a.content, "tooltip")));
				};
				if (a.X) {
					var e = R8b(a);
					d();
				} else _.CIa({
					measure: c,
					Ut: d
				})();
			}
		};
		R8b = function(a) {
			a = a.Ga.nativeElement;
			if ((a.matches ? a.matches("[tabindex], a[href], area[href], iframe, input, textarea, select, button") : Array.from(document.querySelectorAll("[tabindex], a[href], area[href], iframe, input, textarea, select, button")).includes(a)) && s7b(a)) return a;
			var b;
			var c;
			return (c = (b = t7b(a)[0]) != null ? b : t7b(a, true)[0]) != null ? c : a;
		};
		S8b = function(a) {
			return a.FQ() || a.A();
		};
		_.aR = class {
			set jw(a) {
				var b = this.content;
				if (a) {
					this.NV.VHa(this);
					if (a instanceof _.Zh || a instanceof _.od || typeof a === "string") this.content = a;
					else for (let c of Object.keys(a)) this[c] = a[c];
					T8b(this, b);
				} else this.NV.q6(this);
			}
			get jw() {
				return this.content;
			}
			get nq() {
				return this.Gwa ? this.Gwa : this.Ga.nativeElement.tagName === "CFC-ICON" || this.Ga.nativeElement.tagName === "CM-ICON" || this.Ga.nativeElement.hasAttribute("mat-icon-button") ? this.R ? "none" : "label" : "description";
			}
			get R() {
				return this.Ga.nativeElement.getAttribute("aria-label");
			}
			constructor() {
				this.NV = _.m(VQ);
				this.F = _.m(_.VA);
				this.U = _.m(EQ);
				this.control = _.m(_.lD, {
					optional: true,
					host: true
				});
				this.H = _.m(K8b, {
					optional: true,
					host: true
				});
				this.destroyed = false;
				this.Pa = _.m(_.Xf);
				this.Ga = _.m(_.Jf);
				this.cQ = {};
				this.b5 = this.d5 = false;
				this.Gwa = this.bha = undefined;
				this.Fwa = this.ot = false;
				this.zf = new _.Wg();
				this.eC = new _.Wg();
				this.ZB = new _.Wg();
				this.I = false;
				this.FQ = _.Li(false, Object.assign({}, {}, {
					alias: "disabled",
					transform: _.Ml
				}));
				this.A = _.M(false);
				var a;
				var b = (a = _.m($7b, { optional: true })) != null ? a : "bottom";
				a = _.m(a8b, { optional: true });
				var c = _.m(b8b, { optional: true });
				var d = _.m(c8b, { optional: true });
				this.c5 = b || "bottom";
				this.MVa = a !== false;
				this.X = !!c;
				this.Hwa = d != null ? d : Z7b;
				_.Kg(() => {
					this.FQ();
					this.A();
					T8b(this);
				});
			}
			Ba() {
				this.destroyed = true;
				this.NV.q6(this);
			}
			Wb() {
				this.zf.next();
			}
			Rb() {
				var a = this.Ga.nativeElement;
				this.I = true;
				if (this.H) this.A = _.Yi(() => this.H.state().disabled());
				else {
					var b;
					if ((b = this.control) == null ? 0 : b.control) {
						b = this.control.control;
						this.A.set(b.disabled);
						_.dza(b, (c) => {
							if (!this.destroyed) {
								this.A.set(c);
							}
						});
					}
				}
				T8b(this);
				if (this.KVa) {
					b = ["click", "keydown"];
					for (let c of b) a.addEventListener(c, (d) => {
						if (!(d instanceof KeyboardEvent && d.key !== "Enter" && d.key !== "Space")) {
							d.preventDefault();
							d.stopPropagation();
							var e = new d.constructor(d.type, d);
							Object.defineProperty(e, "target", {
								writable: false,
								value: d.target
							});
							document.dispatchEvent(e);
						}
					});
				}
			}
		};
		_.aR.J = function(a) {
			return new (a || _.aR)();
		};
		_.aR.Oa = _.We({
			type: _.aR,
			da: [[
				"",
				"cfcTooltip",
				""
			]],
			Ua: 4,
			Ja: function(a, b) {
				if (a & 1) {
					_.J("keydown.escape", function(c) {
						if (b.NV.x9(b)) {
							c.stopPropagation(), c.preventDefault(), b.NV.El(b), _.RA(b.NV.Mj, "Tooltip dismissed");
						}
					});
				}
				if (a & 2) {
					_.P("cfc-tooltip", true)("cfc-tooltip-disable-user-select-on-touch-device", !b.b5);
				}
			},
			inputs: {
				jw: "cfcTooltip",
				cQ: "cfcTooltipTemplateData",
				d5: "cfcTooltipRich",
				b5: "cfcTooltipDisabled",
				c5: "cfcTooltipPosition",
				bha: "cfcTooltipTapToShow",
				Gwa: "cfcTooltipAriaRelationship",
				ot: "cfcTooltipInteractive",
				LVa: "cfcTooltipInteractiveDialogAriaLabel",
				KVa: "cfcPreventEventPropagation",
				Fwa: "cfcTooltipAlwaysShowOnFocus",
				a5: "cfcTooltipAction",
				FQ: [
					1,
					"disabled",
					"disabledByTemplate"
				]
			},
			features: [_.su]
		});
		_.bR = class {};
		_.bR.J = function(a) {
			return new (a || _.bR)();
		};
		_.bR.qc = _.Ve({ type: _.bR });
		_.bR.oc = _.Dd({ imports: [OQ, PQ] });
		_.cR = class {
			constructor() {
				this.A = _.m(_.jP);
				this.Ga = _.m(_.Jf);
				_.bg({ read: () => {} });
			}
			ib() {
				this.Ga.nativeElement.classList.add(this.A.F);
			}
		};
		_.cR.J = function(a) {
			return new (a || _.cR)();
		};
		_.cR.Oa = _.We({
			type: _.cR,
			da: [[
				"",
				"sdui-token-host",
				""
			]]
		});
		_.yJ.prototype.na = _.ca(179, function(a, b, c, d = true) {
			var e = this.Fa[a];
			if (!e && a !== "impression") return false;
			if (d || !c) var f = new _.hzb(_.fk("syntheticElement"), b);
			else {
				f = new _.hzb(c, b);
				let g = this.fa;
				_.kHa(c, (k) => {
					if (!_.eq(k) || k.nodeType != 1) return false;
					k = _.xJ(g, k);
					return k != null ? (f.setParent(k), true) : false;
				}, false);
			}
			if (this.F) return _.qzb(this, f, new _.Zyb(e)), true;
			a = [];
			if (c) {
				a = _.rzb(this, c);
			}
			this.za(e, f, a);
			return true;
		});
		_.U8b = function() {
			return new _.yib();
		};
		_.dR = class {};
		_.dR.J = function(a) {
			return new (a || _.dR)();
		};
		_.dR.sa = _.Cd({
			token: _.dR,
			factory: _.dR.J
		});
		var BCc;
		var DCc;
		var ECc;
		var FCc;
		var ICc;
		var KX;
		var KCc;
		var LCc;
		var PCc;
		var RCc;
		var SCc;
		var TCc;
		var WCc;
		var aDc;
		var cDc;
		var VX;
		var dDc;
		var fDc;
		var YX;
		var ZX;
		var hDc;
		var $X;
		var aY;
		var EDc;
		var GDc;
		var HDc;
		var JDc;
		var KDc;
		var LDc;
		var NDc;
		var PDc;
		var aEc;
		var bEc;
		var cEc;
		var hEc;
		BCc = function(a, b, c) {
			if (a.namespaceURI !== "http://www.w3.org/1999/xhtml") throw Error("L`" + b + "`" + a.tagName);
			b = b.toLowerCase();
			switch (`${a.tagName} ${b}`) {
				case "A href":
					_.nd(a, c);
					break;
				case "AREA href":
					b = _.md(c);
					b !== undefined && (a.href = b);
					break;
				case "BASE href":
					a.href = _.ed(c);
					break;
				case "BUTTON formaction":
					b = _.md(c);
					b !== undefined && (a.formAction = b);
					break;
				case "EMBED src":
					a.src = _.ed(c);
					break;
				case "FORM action":
					b = _.md(c);
					b !== undefined && (a.action = b);
					break;
				case "IFRAME src":
					a.src = _.ed(c).toString();
					break;
				case "IFRAME srcdoc":
					a.srcdoc = _.qd(c);
					break;
				case "IFRAME sandbox": throw Error("M");
				case "INPUT formaction":
					b = _.md(c);
					b !== undefined && (a.formAction = b);
					break;
				case "LINK href": throw Error("N");
				case "LINK rel": throw Error("O");
				case "OBJECT data":
					a.data = _.ed(c);
					break;
				case "SCRIPT src":
					_.td(a, c);
					break;
				default:
					if (/^on./.test(b)) throw Error("P`" + b);
					a.setAttribute(b, c);
			}
		};
		DCc = function(a, b, c) {
			var d = b.toLowerCase();
			if (CCc.indexOf(d) !== -1 || d.indexOf("on") === 0) throw Error("J");
			a.setAttribute(b, c);
		};
		ECc = function(a, b) {
			return new _.ef((c) => {
				var d = (...f) => c.next(f.length === 1 ? f[0] : f);
				var e = a(d);
				return _.Ze(b) ? () => b(d, e) : undefined;
			});
		};
		_.Uv = function(a) {
			this.id = a;
		};
		FCc = function(a) {
			return (b) => new _.ef((c) => {
				var d = b.subscribe({
					next(e) {
						a.run(() => {
							c.next(e);
						});
					},
					error(e) {
						a.run(() => {
							c.error(e);
						});
					},
					complete() {
						a.run(() => {
							c.complete();
						});
					}
				});
				return () => {
					d.unsubscribe();
				};
			});
		};
		GCc = function() {
			var a = window;
			var b = a;
			for (; b !== a.top;) b.parent.queueMicrotask(() => {}), b = b.parent;
			return b;
		};
		ICc = function() {
			var a = GCc();
			if (!a.panXChannelServices) {
				a.panXChannelServices = new HCc();
			}
			return a.panXChannelServices;
		};
		KX = function(a, b) {
			this.A = a;
			this.F = b;
		};
		JCc = function() {
			this.A = [];
			this.F = [];
		};
		KCc = function() {};
		LCc = function() {
			return 0;
		};
		PCc = function(a, b, c) {
			return (d) => {
				if (d === "") throw Error("ch");
				d = c ? new MCc(d, a, b, c) : new NCc(d, a, b);
				return new OCc(d);
			};
		};
		RCc = function(a = "xapp-channel") {
			var b = new QCc("client");
			if (typeof a !== "string") b.connectXappChannel(a);
			else {
				let c = _.Cs(a);
				if (!c) throw Error("fh`" + a);
				b.connectXappChannel(c);
			}
			return new OCc(b, { parameters: {} });
		};
		SCc = function() {
			var a = {
				name: "server",
				symbol: "xapp-channel"
			};
			var b = {};
			if (a instanceof QCc) return new OCc(a, { parameters: b });
			var c = new QCc(a.name);
			_.wk(a.symbol, c);
			return new OCc(c, { parameters: b });
		};
		TCc = function(a, b) {
			_.x(function* () {
				var c = yield b;
				LX({
					metadata: Object.assign({}, c, { unit: 1 }),
					value: a
				});
			});
		};
		WCc = function() {
			var a = new UCc((b) => new UCc((c) => b(c)));
			VCc.add(a);
			return (b, c) => a.get(b).get(c);
		};
		_.MX = function(a) {
			return a ? `sdui-${a.trim()}` : null;
		};
		_.NX = function(a) {
			return a ? a.split(" ").filter((b) => b).map((b) => _.MX(b)).join(" ") : null;
		};
		_.YCc = function(a) {
			return new _.OX().setValue(_.PX(new _.QX(), _.XCc(new _.RX(), a)));
		};
		aDc = function(a) {
			return new _.OX().setValue(_.PX(new _.QX(), _.$Cc(new _.RX(), a)));
		};
		bDc = function(a) {
			switch (a.type) {
				case "int64":
				case "uint64": return a.value.toString();
				case "double": return Number.isSafeInteger(a.value) ? BigInt(a.value).toString() : undefined;
				case "string": return "s:" + a.value;
				case "bool": return `b:${a.value}`;
			}
		};
		cDc = function(a, b) {
			return function* () {
				for (let c of a) yield b(c);
			}();
		};
		SX = function() {
			return {
				type: "null",
				value: null
			};
		};
		_.TX = function(a) {
			return {
				type: "string",
				value: a
			};
		};
		_.UX = function(a) {
			if (typeof a === "number" && !Number.isSafeInteger(a)) throw Error("rh`" + a + "`" + a);
			return {
				type: "int64",
				value: BigInt(a)
			};
		};
		VX = function(a) {
			if (typeof a === "number" && !Number.isSafeInteger(a)) throw Error("sh`" + a + "`" + a);
			var b = BigInt(a);
			if (b < BigInt(0)) throw Error("th`" + a);
			return {
				type: "uint64",
				value: b
			};
		};
		WX = function(a) {
			return {
				type: "double",
				value: a
			};
		};
		_.XX = function(a) {
			return {
				type: "bool",
				value: a
			};
		};
		dDc = function(a) {
			return {
				type: "bytes",
				value: a
			};
		};
		fDc = function(a) {
			return {
				type: "duration",
				value: typeof a === "string" ? eDc(a) : a
			};
		};
		YX = function(a) {
			return {
				type: "timestamp",
				value: typeof a === "string" ? gDc(a) : a
			};
		};
		ZX = function(a) {
			return {
				type: "list",
				value: { elements: a }
			};
		};
		hDc = function(a) {
			return {
				type: "error",
				value: a
			};
		};
		$X = function(a) {
			return {
				type: "type",
				value: a
			};
		};
		iDc = function(a, b) {
			if (a.size !== b.size) return false;
			for (let [d, e] of a.entries()) {
				var c = d;
				a = e;
				if (!b.contains(c)) return false;
				c = b.get(c);
				if (!aY(a, c)) return false;
			}
			return true;
		};
		aY = function(a, b) {
			var c = a.type;
			var d = b.type;
			if (c === "error") throw a.value;
			if (d === "error") throw b.value;
			switch (c) {
				case "int64":
				case "uint64":
				case "double": return d === "int64" || d === "uint64" || d === "double" ? jDc(a.value, b.value) === 0 : false;
				case "string": return d === "string" && a.value === b.value;
				case "bool": return d === "bool" && a.value === b.value;
				case "bytes": return d === "bytes" && _.rPa(a.value, b.value);
				case "duration": return d === "duration" && a.value.equals(b.value);
				case "timestamp": return d === "timestamp" && a.value.equals(b.value);
				case "map": return d === "map" && iDc(a.value, b.value);
				case "list":
					if (c = d === "list") a: if (a = a.value.elements, b = b.value.elements, a.length !== b.length) c = false;
					else {
						for (c = 0; c < a.length; c++) if (!aY(a[c], b[c])) {
							c = false;
							break a;
						}
						c = true;
					}
					return c;
				case "null": return d === "null";
				case "type": return d === "type" && a.value === b.value;
				default: _.sb(c, undefined);
			}
		};
		jDc = function(a, b) {
			if (typeof a === typeof b) return a === b ? 0 : a < b ? -1 : 1;
			if (typeof a === "bigint") return -jDc(b, a);
			if (Number.isNaN(a)) return -1;
			if (a === Number.POSITIVE_INFINITY) return 1;
			if (a === Number.NEGATIVE_INFINITY) return -1;
			if (Number.isSafeInteger(a)) return a = BigInt(a), a === b ? 0 : a < b ? -1 : 1;
			b = Number(b);
			return a < b ? -1 : a > b ? 1 : 0;
		};
		kDc = function(a) {
			return a.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
		};
		lDc = function(a, b, c) {
			switch (a.type) {
				case "int64":
				case "uint64":
				case "double":
					if (b.type === "int64" || b.type === "uint64" || b.type === "double") return jDc(a.value, b.value);
					break;
				case "string":
					if (b.type === "string") return a.value < b.value ? -1 : a.value > b.value ? 1 : 0;
					break;
				case "bool":
					if (b.type === "bool") return a.value === b.value ? 0 : a.value ? 1 : -1;
					break;
				case "bytes":
					if (b.type === "bytes") return a = _.Is(a.value), b = _.Is(b.value), a < b ? -1 : a > b ? 1 : 0;
					break;
				case "duration":
					if (b.type === "duration") return a = a.value, b = b.value, a.A < b.A ? -1 : a.A > b.A ? 1 : 0;
					break;
				case "timestamp": if (b.type === "timestamp") return a = a.value, b = b.value, a.seconds !== b.seconds ? a.seconds < b.seconds ? -1 : 1 : a.nanos - b.nanos;
			}
			throw new bY(c, a, b);
		};
		oDc = function(a) {
			if (a < mDc || a > nDc) throw new cY("Int64 overflow");
			return a;
		};
		qDc = function(a) {
			if (a < BigInt(0) || a > pDc) throw new cY("Uint64 overflow");
			return a;
		};
		rDc = function(a, b) {
			var c = a.type;
			var d = b.type;
			if (c === d) {
				switch (c) {
					case "int64":
						if (d === "int64") return _.UX(oDc(a.value + b.value));
						break;
					case "uint64":
						if (d === "uint64") return VX(qDc(a.value + b.value));
						break;
					case "double":
						if (d === "double") return WX(a.value + b.value);
						break;
					case "string":
						if (d === "string") return _.TX(a.value + b.value);
						break;
					case "bytes":
						if (d === "bytes") return a = _.ep(a.value), b = _.ep(b.value), c = new Uint8Array(a.length + b.length), c.set(a), c.set(b, a.length), dDc(_.cb(c));
						break;
					case "duration":
						if (d === "duration") return fDc(a.value.plus(b.value));
						break;
					case "list":
						if (d === "list") return ZX([...a.value.elements, ...b.value.elements]);
						break;
					case "bool":
					case "timestamp":
					case "map":
					case "null":
					case "error":
					case "type": break;
					default: _.sb(c, undefined);
				}
				throw new bY("_+_", a, b);
			}
			if (c === "timestamp" && d === "duration") return YX(a.value.plus(b.value));
			if (c === "duration" && d === "timestamp") return YX(b.value.plus(a.value));
			throw new bY("_+_", a, b);
		};
		sDc = function(a, b) {
			var c = a.type;
			var d = b.type;
			if (c === d) {
				switch (c) {
					case "int64":
						if (d === "int64") return _.UX(oDc(a.value - b.value));
						break;
					case "uint64":
						if (d === "uint64") return VX(qDc(a.value - b.value));
						break;
					case "double":
						if (d === "double") return WX(a.value - b.value);
						break;
					case "timestamp":
						if (d === "timestamp") return a = a.value, b = b.value, b = dY((a.seconds - b.seconds) * BigInt(1e9) + BigInt(a.nanos - b.nanos)), fDc(b);
						break;
					case "duration":
						if (d === "duration") return fDc(a.value.minus(b.value));
						break;
					case "bool":
					case "string":
					case "bytes":
					case "list":
					case "map":
					case "null":
					case "error":
					case "type": break;
					default: _.sb(c, undefined);
				}
				throw new bY("_-_", a, b);
			}
			if (c === "timestamp" && d === "duration") return YX(a.value.minus(b.value));
			throw new bY("_-_", a, b);
		};
		tDc = function(a) {
			switch (a.type) {
				case "string": return a;
				case "bool": return _.TX(a.value ? "true" : "false");
				case "int64":
				case "uint64":
				case "double": return _.TX(a.value.toString());
				case "bytes": try {
					return _.TX(_.Is(a.value));
				} catch (b) {
					throw new eY("Invalid UTF-8", { cause: b });
				}
				case "duration":
				case "timestamp": return _.TX(a.value.toString());
				default: throw new bY("string", a);
			}
		};
		uDc = function(a) {
			if (a.type === "int64") return a;
			if (a.type === "uint64") {
				if (a.value > nDc) throw new cY("range");
				return _.UX(a.value);
			}
			if (a.type === "double") {
				a = a.value;
				if (!Number.isFinite(a) || a > nDc || a <= mDc) throw new cY("range");
				return _.UX(BigInt(Math.trunc(a)));
			}
			if (a.type === "string") {
				a = BigInt(a.value);
				if (a > nDc || a < mDc) throw new cY("range");
				return _.UX(a);
			}
			if (a.type === "timestamp") return _.UX(a.value.seconds);
			throw new bY("int", a);
		};
		vDc = function(a) {
			if (a.type === "uint64") return a;
			if (a.type === "int64") {
				if (a.value < 0) throw new cY("range");
				return VX(a.value);
			}
			if (a.type === "double") {
				a = a.value;
				if (!Number.isFinite(a)) throw new cY("range");
				if (a < 0 || a > pDc) throw new cY("range");
				return VX(BigInt(Math.trunc(a)));
			}
			if (a.type === "string") try {
				let b = BigInt(a.value);
				if (b < 0) throw new cY("range");
				return VX(b);
			} catch (b) {
				throw new eY("Invalid uint", { cause: b });
			}
			throw new bY("uint", a);
		};
		wDc = function(a, b) {
			return _.XX(a.value.elements.some((c) => aY(c, b)));
		};
		xDc = function(a) {
			switch (a.type) {
				case "list": return _.UX(a.value.elements.length);
				case "map": return _.UX(a.value.size);
				case "string": return a = [...a.value].length, _.UX(a);
				case "bytes": return _.UX(a.value.sizeBytes());
				default: throw new bY("size", a);
			}
		};
		yDc = function(a, b) {
			if (a.type !== "list") throw new bY("join", a);
			var c = "";
			if (b) {
				if (b.type !== "string") throw new bY("join", a, b);
				c = b.value;
			}
			b = [];
			for (let d of a.value.elements) {
				if (d.type !== "string") throw new bY("join", a);
				b.push(d.value);
			}
			return _.TX(b.join(c));
		};
		zDc = function(a, b, c) {
			if (a.type !== "string") throw new bY("substring", a, b, c != null ? c : SX());
			if (b.type !== "int64" && b.type !== "uint64") throw new bY("substring", a, b, c != null ? c : SX());
			if (c && c.type !== "int64" && c.type !== "uint64") throw new bY("substring", a, b, c);
			a = [...a.value];
			var d = a.length;
			b = Number(b.value);
			c = c ? Number(c.value) : d;
			if (b < 0 || c < 0 || b > d || c > d || b > c) throw new eY(`String index out of range: [${b}, ${c}]`);
			return _.TX(a.slice(b, c).join(""));
		};
		ADc = function(a, b, c, d) {
			if (a.type === "string" && b.type === "string" && c.type === "string" && (d === undefined || d.type === "int64")) {
				let f = a.value;
				b = b.value;
				c = c.value;
				d = d !== undefined ? Number(d.value) : -1;
				if (d === 0) return a;
				if (d < 0) return _.TX(f.replaceAll(b, c));
				a = "";
				let g = 0;
				let k = 0;
				if (b === "") {
					a += c;
					k++;
					for (var e of f) if (k < d) {
						a += e + c;
						k++;
						g += e.length;
					} else {
						a += f.slice(g);
						break;
					}
					return _.TX(a);
				}
				for (; k < d;) {
					e = f.indexOf(b, g);
					if (e === -1) break;
					a += f.slice(g, e) + c;
					g = e + b.length;
					k++;
				}
				a += f.slice(g);
				return _.TX(a);
			}
			throw new bY("replace", a, b, c, d);
		};
		BDc = function(a, b, c) {
			if (a.type !== "string" || b.type !== "string") throw new bY("split", a, b, c != null ? c : SX());
			var d = a.value;
			var e = b.value;
			if (c && c.type !== "int64" && c.type !== "uint64") throw new bY("split", a, b, c);
			if (c) {
				var f = BigInt(c.value);
				if (f > BigInt(2147483647) || f < BigInt(-2147483648)) throw new eY(`Limit value "${c.value}" is out of 32-bit integer range.`);
				c = Number(f);
			} else c = 2147483647;
			if (c === 0) return ZX([]);
			if (c === 1) return ZX([a]);
			if (c < 0) return ZX(a.value.split(b.value).map(_.TX));
			if (e === "") return d = [...d], e = d.slice(0, c - 1), e.push(d.slice(c - 1).join("")), ZX(e.map(_.TX));
			a = 0;
			b = [];
			for (f = 0; f < c - 1; f++) {
				let g = d.indexOf(e, a);
				if (g === -1) break;
				b.push(d.slice(a, g));
				a = g + e.length;
			}
			b.push(d.slice(a));
			return ZX(b.map(_.TX));
		};
		CDc = function(a) {
			if (a.type !== "string") throw new bY("lowerAscii", a);
			return _.TX(a.value.replace(/[A-Z]/g, (b) => b.toLowerCase()));
		};
		DDc = function(a) {
			if (a.type !== "string") throw new bY("upperAscii", a);
			return _.TX(a.value.replace(/[a-z]/g, (b) => b.toUpperCase()));
		};
		EDc = function(a) {
			switch (_.jj(a, _.fY)) {
				case 1: return SX();
				case 2: return _.XX(_.Ao(a, 2, _.fY));
				case 3: return _.UX(BigInt(_.Ys(a, _.Ls(a, _.fY, 3))));
				case 4: return VX(BigInt(_.Zs(a, _.Ls(a, _.fY, 4))));
				case 5: return WX(_.zo(a, 5, _.fY));
				case 6: return _.TX(_.qj(a, 6, _.fY));
				case 7: return dDc(_.Ep(a, 7, _.fY));
				default: throw new gY(`Constant type "${_.jj(a, _.fY)}" is not supported.`);
			}
		};
		GDc = function(a) {
			var b = [];
			var c = [];
			var d = [];
			var e = [];
			for (let f of _.mj(a, FDc, 1020, _.oj())) if (a = f.getAction()) {
				if (_.vn(f, 4)) {
					b.push(a);
				}
				if (_.vn(f, 5)) {
					c.push(a);
				}
				if (_.vn(f, 6)) {
					d.push(a);
				}
				if (_.vn(f, 7)) {
					e.push(a);
				}
			}
			return {
				KKb: b,
				LKb: c,
				DDb: d,
				Lxb: e
			};
		};
		HDc = function(a, b, c, d, e) {
			return (b == null ? undefined : b.R) === c ? b : _.Fu(a, c, {
				Pa: _.Xi({
					parent: d,
					vd: []
				}),
				Oy: e
			});
		};
		JDc = function(a) {
			if (a && a <= 0) throw new IDc();
		};
		KDc = function({ metadata: a }) {
			LX({
				value: Date.now(),
				metadata: Object.assign({}, a, { unit: 201 })
			});
		};
		LDc = function(a) {
			_.x(function* () {
				yield Promise.resolve();
				a();
			});
		};
		NDc = function(a) {
			return new Promise((b, c) => {
				if (a == null ? 0 : a.requestTimeout) {
					setTimeout(() => {
						LX();
						c(new MDc());
					}, a.requestTimeout);
				}
			});
		};
		PDc = function(a) {
			var b = a.ug();
			return ODc.some((c) => b.endsWith(`.${c}`));
		};
		QDc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.yzb.prototype.Gn = _.ca(180, function() {
			return _.sn(this, _.Ny, 1);
		});
		_.aG.prototype.yja = _.ca(167, function() {
			return _.fj(this, QDc, 20, _.Br);
		});
		_.hx.prototype.qm = _.ca(55, function() {
			return _.zn(this, 1);
		});
		_.Ar.prototype.qm = _.ca(54, function() {
			return _.zn(this, 3);
		});
		_.kH.prototype.qm = _.ca(53, function() {
			return _.sn(this, _.Vub, 1);
		});
		var RDc = {
			Tl: ",",
			Jm: ".",
			oo: "%",
			un: "0",
			bp: "+",
			rn: "-",
			jo: "E",
			po: "‰",
			qn: "∞",
			Zo: "NaN",
			io: "#,##0.###",
			ep: "#E0",
			ap: "#,##0\xA0%",
			Xo: "#,##0.00\xA0¤",
			nn: "EUR"
		};
		var SDc = {
			Tl: ",",
			Jm: " ",
			oo: "%",
			un: "0",
			bp: "+",
			rn: "-",
			jo: "E",
			po: "‰",
			qn: "∞",
			Zo: "NaN",
			io: "#,##0.###",
			ep: "#E0",
			ap: "#,##0\xA0%",
			Xo: "#,##0.00\xA0¤",
			nn: "EUR"
		};
		var hY = function(a) {
			var b = new _.lw();
			return _.gt(b, 1, a);
		};
		var iY = function(a, b) {
			return _.Uc(a, 2, b);
		};
		var TDc = [
			0,
			_.Ft,
			_.Jt,
			_.Rt,
			_.AUa
		];
		var UDc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getType() {
				return _.Lm(this, 1);
			}
		};
		var VDc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getTitle() {
				return _.l(this, 1);
			}
		};
		var WDc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getMetadata() {
				return _.Z(this, VDc, 2);
			}
			A(a) {
				return _.ln(this, VDc, 2, a);
			}
			F() {
				return _.sn(this, VDc, 2);
			}
			kR() {
				return _.Z(this, UDc, 3);
			}
		};
		var XDc = _.Mc(function(a, b, c) {
			if (a.F !== 1) return false;
			_.Nc(b, c, _.TPa(a.A));
			return true;
		}, _.Ida, _.rQa);
		var YDc = _.Mc(function(a, b, c) {
			if (a.F !== 5) return false;
			var d = _.vt(a.A);
			a = (d >> 31 << 1) + 1;
			var e = d >>> 23 & 255;
			d &= 8388607;
			a = e == 255 ? d ? NaN : a * Infinity : e == 0 ? a * 1401298464324817e-60 * d : a * Math.pow(2, e - 150) * (d + 8388608);
			_.Nc(b, c, a === 0 ? undefined : a);
			return true;
		}, function(a, b, c) {
			b = _.ub(b);
			if (b != null) {
				_.Oc(a, c, 5), a = a.A, c = _.Hda || (_.Hda = new DataView(new ArrayBuffer(8))), c.setFloat32(0, +b, true), _.pb = 0, _.ob = c.getUint32(0, true), _.Pc(a, _.ob);
			}
		}, _.qQa);
		var ZDc = _.Mc(function(a, b, c, d) {
			if (a.F !== 0) {
				a = false;
			} else {
				_.Qs(b, c, d, _.ut(a.A, _.nba)), a = true;
			}
			return a;
		}, _.Nda, _.oQa);
		var $Dc = _.Gda(_.Wda, function(a, b, c) {
			b = _.Fda(_.Db, b);
			if (b != null && b.length) {
				c = _.fQa(a, c);
				for (let d = 0; d < b.length; d++) _.Rc(a.A, b[d]);
				_.gQa(a, c);
			}
		}, _.kQa);
		var jY = _.Gda(function(a, b, c) {
			if (a.F !== 2) return false;
			a = _.$Pa(a);
			_.Dc(b, b[_.Ra] | 0, c).push(a);
			return true;
		}, function(a, b, c) {
			b = _.Fda(_.Xb, b);
			if (b != null) for (let g = 0; g < b.length; g++) {
				var d = a;
				var e = c;
				var f = b[g];
				if (f != null) {
					_.Qda(d, e, _.laa(f, false));
				}
			}
		}, _.jQa);
		var kY = [
			true,
			_.zt,
			_.zt
		];
		_.Uv.prototype.toString = function() {
			return this.id;
		};
		aEc = [
			0,
			_.du,
			-1
		];
		bEc = [
			0,
			_.Ft,
			-2
		];
		cEc = ({ destination: a, origin: b, token: c, channelName: d = "ZNWN1d", onMessage: e }) => {
			if (b === "*") throw Error("Ic");
			var f = _.Stb(e);
			a.postMessage(c ? {
				n: d,
				t: c
			} : d, b, [f.port2]);
			return _.Utb(f.port1, e);
		};
		dEc = [
			0,
			_.Jt,
			-1,
			_.Rt,
			_.fv
		];
		eEc = [
			0,
			[0, _.fv],
			_.yt,
			_.At([
				0,
				_.Jt,
				-1,
				_.Vt,
				_.ev
			]),
			[
				0,
				_.Jt,
				_.ev
			],
			_.fv,
			_.ev
		];
		fEc = [
			0,
			_.k1b,
			_.St,
			[
				0,
				_.Jt,
				[
					0,
					_.Jt,
					_.Rt,
					[
						0,
						_.yt,
						kY,
						_.Rt,
						_.fv,
						_.Rt,
						dEc
					]
				],
				[
					0,
					_.Jt,
					-5
				],
				_.zt,
				-1
			],
			_.St,
			eEc
		];
		gEc = [
			0,
			_.l1b,
			_.St,
			[
				0,
				_.Jt,
				[
					0,
					_.Jt,
					_.Rt,
					[
						0,
						_.yt,
						kY,
						_.Rt,
						_.fv,
						_.Rt,
						dEc,
						_.fv,
						dEc
					]
				],
				[
					0,
					_.Jt,
					-2
				],
				_.zt,
				-1
			],
			_.St,
			eEc
		];
		hEc = [
			0,
			_.Jt,
			[
				0,
				_.zt,
				-3,
				[0, _.zt]
			],
			[
				0,
				_.dP,
				_.Ut,
				_.St,
				[0, fEc],
				_.St,
				[0, fEc],
				_.St,
				[0, fEc],
				_.St,
				[0, gEc],
				_.St,
				[0, gEc],
				_.St,
				[0, gEc]
			],
			_.ev,
			_.Gt
		];
		_.XCc = function(a, b) {
			return _.ft(a, 2, _.fY, b);
		};
		_.$Cc = function(a, b) {
			return _.PPa(a, 3, _.fY, b);
		};
		_.RX = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.fY = [
			1,
			2,
			3,
			4,
			5,
			6,
			7,
			8,
			9
		];
		_.lY = class extends _.h {
			constructor(a) {
				super(a);
			}
			getName() {
				return _.l(this, 1);
			}
		};
		var iEc;
		iEc = class extends _.h {
			constructor(a) {
				super(a);
			}
			A() {
				return _.Z(this, _.QX, 7);
			}
			Mx(a) {
				return _.ln(this, _.QX, 7, a);
			}
			Tm() {
				return _.In(this, 7);
			}
		};
		_.jEc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getId() {
				return _.Ys(this, 1);
			}
			getValue() {
				return _.Z(this, _.QX, 4);
			}
			setValue(a) {
				return _.ln(this, _.QX, 4, a);
			}
			xc() {
				return _.sn(this, _.QX, 4);
			}
		};
		_.kEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.lEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.mEc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Fn() {
				return _.Z(this, _.QX, 1);
			}
			Gn() {
				return _.sn(this, _.QX, 1);
			}
			getFunction() {
				return _.l(this, 2);
			}
		};
		_.PX = function(a, b) {
			return _.Ap(a, 3, _.mY, b);
		};
		_.QX = class extends _.h {
			constructor(a) {
				super(a);
			}
			getId() {
				return _.Ys(this, 2);
			}
		};
		_.nEc = [2, 3];
		_.mY = [
			3,
			4,
			5,
			6,
			7,
			8,
			9
		];
		_.nY = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.oY = [
			1,
			2,
			3,
			4,
			5,
			6,
			7
		];
		var oEc = [
			0,
			_.fY,
			_.GQa,
			_.It,
			_.yQa,
			ZDc,
			_.Bt,
			_.Kt,
			_.EQa,
			_.St,
			_.jv,
			_.St,
			_.du
		];
		var pY = [0, _.Jt];
		var pEc = [
			0,
			_.Jt,
			() => qY,
			_.Jt,
			() => qY,
			-3,
			_.Jt
		];
		var qEc = [
			0,
			_.nEc,
			_.Dt,
			_.Kt,
			_.St,
			() => qY,
			() => qY,
			_.Ht
		];
		var rEc = [
			0,
			_.Jt,
			_.Rt,
			() => qEc
		];
		var sEc = [
			0,
			_.Rt,
			() => qY,
			$Dc
		];
		var tEc = [
			0,
			() => qY,
			_.Jt,
			_.Rt,
			() => qY
		];
		var uEc = [
			0,
			() => qY,
			_.Jt,
			_.Ht
		];
		var qY = [
			0,
			_.mY,
			1,
			_.Dt,
			_.St,
			oEc,
			_.St,
			pY,
			_.St,
			() => uEc,
			_.St,
			() => tEc,
			_.St,
			() => sEc,
			_.St,
			() => rEc,
			_.St,
			() => pEc
		];
		var wEc = [
			0,
			_.Rt,
			() => vEc
		];
		var vEc = [
			0,
			_.oY,
			_.GQa,
			_.Bt,
			_.Kt,
			_.It,
			_.St,
			() => xEc,
			_.St,
			() => wEc,
			_.St,
			qY
		];
		var xEc = [
			0,
			_.yt,
			_.At(() => vEc)
		];
		var yEc = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
			lja() {
				return _.Pm(this, 2);
			}
			Bl() {
				return _.l(this, 1);
			}
			hasLabel() {
				return _.zn(this, 1);
			}
			Bq() {
				return _.l(this, 3);
			}
			Mw() {
				return _.Pm(this, 1001);
			}
			xC(a) {
				return _.Mj(this, 1001, a);
			}
		};
		var zEc = [
			-500,
			_.zt,
			_.Gt,
			_.zt,
			-2,
			_.Gt,
			_.zt,
			_.Gt,
			XDc,
			-2,
			989,
			_.Gt
		];
		_.AEc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getId() {
				return _.l(this, 1);
			}
			qm() {
				return _.zn(this, 1);
			}
		};
		var BEc = [0, _.zt];
		var CEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var DEc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.l(this, 1);
			}
			Fe() {
				return _.zn(this, 1);
			}
			getLanguage() {
				return _.l(this, 3);
			}
			sAa() {
				return _.Lm(this, 5);
			}
		};
		var EEc = [
			0,
			_.zt,
			1,
			_.zt,
			_.Gt,
			_.Ut,
			1,
			_.Ut,
			_.Gt
		];
		_.FEc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getText() {
				return _.l(this, 1);
			}
			setText(a) {
				return _.Lj(this, 1, a);
			}
		};
		var GEc = [0, _.zt];
		_.HEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.IEc = [5, 6];
		var JEc = [
			0,
			_.IEc,
			4,
			_.St,
			[
				0,
				_.zt,
				-1,
				_.Gt,
				-1
			],
			_.St,
			[
				0,
				_.zt,
				-1
			]
		];
		_.KEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var LEc = [0, _.zt];
		_.MEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var NEc = [0, _.zt];
		var OEc = [
			0,
			_.zt,
			qY,
			pY
		];
		var PEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var QEc = [
			0,
			_.zt,
			-1,
			[
				0,
				1,
				_.zt
			]
		];
		var REc = [0, _.Tt];
		var SEc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getUrl() {
				return _.l(this, 1);
			}
			getData() {
				return _.Z(this, _.nY, 2);
			}
			ze() {
				return _.sn(this, _.nY, 2);
			}
		};
		var TEc = [
			0,
			_.zt,
			xEc
		];
		var UEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var VEc = [0, _.zt];
		var WEc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Z(this, _.QX, 1);
			}
			setValue(a) {
				return _.ln(this, _.QX, 1, a);
			}
			xc() {
				return _.sn(this, _.QX, 1);
			}
		};
		var XEc = [0, qY];
		_.YEc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.YEc.prototype.NY = _.ba(196);
		var ZEc = [
			0,
			_.zt,
			pY
		];
		var $Ec = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var aFc = [
			0,
			_.zt,
			_.yt,
			_.At([
				0,
				[
					1,
					2,
					3,
					4
				],
				_.Kt,
				_.yQa,
				_.Bt,
				_.It
			])
		];
		var bFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getData() {
				return _.Z(this, _.no, 1);
			}
			ze() {
				return _.sn(this, _.no, 1);
			}
		};
		var cFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getText() {
				return _.qj(this, 1, rY);
			}
			setText(a) {
				return _.mt(this, 1, rY, a);
			}
			getData() {
				return _.fj(this, bFc, 3, rY);
			}
			ze() {
				return _.Dr(this, bFc, 3, rY);
			}
			getMetadata() {
				return _.Z(this, _.no, 4);
			}
			A(a) {
				return _.ln(this, _.no, 4, a);
			}
			F() {
				return _.sn(this, _.no, 4);
			}
		};
		var rY = [
			1,
			2,
			3
		];
		cFc.prototype.I = "AsWIme";
		var dFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getPrompt() {
				return _.l(this, 1);
			}
			setPrompt(a) {
				return _.Lj(this, 1, a);
			}
			getMetadata() {
				return _.Z(this, $Ec, 4);
			}
			A(a) {
				return _.ln(this, $Ec, 4, a);
			}
			F() {
				return _.sn(this, $Ec, 4);
			}
			Gg() {
				return _.mj(this, cFc, 9, _.oj());
			}
		};
		var eFc = [
			0,
			[
				3,
				4,
				5,
				6
			],
			_.Jt,
			_.Vt,
			_.St,
			[
				0,
				_.Jt,
				_.zt,
				-2
			],
			_.St,
			[0, [
				0,
				_.Jt,
				-1
			]],
			_.St,
			[
				0,
				[
					2,
					3,
					4
				],
				[
					0,
					_.Jt,
					-2,
					_.zt
				],
				_.St,
				[0],
				_.St,
				[0],
				_.St,
				[0, [0, _.Jt]]
			],
			_.St,
			[
				0,
				[
					0,
					_.Jt,
					-2
				],
				_.Jt
			]
		];
		var fFc = [
			0,
			_.Jt,
			-1
		];
		var gFc = [
			0,
			[
				0,
				jY,
				_.xQa,
				-3
			],
			_.Rt,
			() => gFc
		];
		var hFc = [
			0,
			_.Jt,
			-1,
			1,
			jY,
			_.Jt,
			-1,
			jY
		];
		var iFc = [
			0,
			_.Jt,
			-1
		];
		var jFc = [
			0,
			_.Jt,
			-2
		];
		var kFc = [
			0,
			_.Jt,
			_.Vt,
			_.du,
			_.Jt,
			jY,
			_.Rt,
			TDc,
			_.Rt,
			[
				0,
				_.du,
				_.Jt,
				_.ev,
				_.Vt,
				_.ev
			],
			_.Jt,
			_.Rt,
			[
				0,
				[
					1,
					2,
					3
				],
				_.Kt,
				_.St,
				jFc,
				_.Kt
			],
			_.Jt,
			_.du,
			jY
		];
		var lFc = [
			0,
			_.Jt,
			-1
		];
		_.OX = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Z(this, _.QX, 1);
			}
			setValue(a) {
				return _.ln(this, _.QX, 1, a);
			}
			xc() {
				return _.sn(this, _.QX, 1);
			}
		};
		var mFc = [0, qY];
		_.sY = class extends _.h {
			constructor(a) {
				super(a);
			}
			getId() {
				return _.l(this, 1);
			}
			qm() {
				return _.zn(this, 1);
			}
			getValue() {
				return _.Z(this, _.OX, 2);
			}
			setValue(a) {
				return _.ln(this, _.OX, 2, a);
			}
			xc() {
				return _.sn(this, _.OX, 2);
			}
		};
		var nFc = [
			0,
			_.zt,
			mFc
		];
		_.oFc = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
			getId() {
				return _.l(this, 6);
			}
			qm() {
				return _.zn(this, 6);
			}
		};
		_.pFc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var qFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getText() {
				return _.l(this, 1);
			}
			setText(a) {
				return _.Lj(this, 1, a);
			}
		};
		_.rFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getType() {
				return _.l(this, 1);
			}
			getName() {
				return _.l(this, 2);
			}
		};
		var sFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getId() {
				return _.Xs(this, 1);
			}
			qm() {
				return _.Fb(_.tn(this, 1)) != null;
			}
			getMetadata() {
				return _.l(this, 2);
			}
			A(a) {
				return _.Lj(this, 2, a);
			}
			F() {
				return _.zn(this, 2);
			}
			o5() {
				return _.In(this, 7);
			}
		};
		var FDc;
		var UFc;
		_.tFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			pR() {
				return _.Z(this, _.tY, 1);
			}
			setHost(a) {
				return _.ln(this, _.tY, 1, a);
			}
		};
		_.vY = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Z(this, _.lY, 1);
			}
			setValue(a) {
				return _.ln(this, _.lY, 1, a);
			}
			xc() {
				return _.sn(this, _.lY, 1);
			}
			tB() {
				return _.mj(this, _.uY, 2, _.oj());
			}
			lE() {
				return _.l(this, 8);
			}
			Wy() {
				return _.Pm(this, 6);
			}
		};
		_.vY.prototype.Zr = _.ba(201);
		_.vY.prototype.tja = _.ba(103);
		_.uFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.l(this, 3);
			}
			Fe() {
				return _.zn(this, 3);
			}
			Xy() {
				return _.mj(this, _.tY, 4, _.oj());
			}
			zo() {
				return _.mj(this, _.uY, 8, _.oj());
			}
		};
		_.vFc = function(a) {
			return _.mj(a, _.tY, 1, _.oj());
		};
		_.wY = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.wFc = function(a) {
			return _.mj(a, _.tY, 1, _.oj());
		};
		_.xFc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.yFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getHeader() {
				return _.Pm(this, 1);
			}
			setHeader(a) {
				return _.Mj(this, 1, a);
			}
			hasHeader() {
				return _.vn(this, 1);
			}
			Sb() {
				return _.Z(this, _.tY, 2);
			}
			Fe() {
				return _.sn(this, _.tY, 2);
			}
		};
		_.zFc = function(a) {
			return _.mj(a, _.tY, 1, _.oj());
		};
		AFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getHeader() {
				return _.Z(this, _.tY, 3);
			}
			setHeader(a) {
				return _.ln(this, _.tY, 3, a);
			}
			hasHeader() {
				return _.sn(this, _.tY, 3);
			}
			I7() {
				return _.Z(this, _.tY, 4);
			}
			aBa() {
				return _.sn(this, _.tY, 4);
			}
		};
		_.xY = class extends _.h {
			constructor(a) {
				super(a);
			}
			zo() {
				return _.mj(this, _.uY, 1, _.oj());
			}
			Bl() {
				return _.l(this, 2);
			}
			hasLabel() {
				return _.zn(this, 2);
			}
		};
		_.BFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 1);
			}
			Fe() {
				return _.sn(this, _.tY, 1);
			}
		};
		_.CFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 1);
			}
			Fe() {
				return _.sn(this, _.tY, 1);
			}
			getValue() {
				return _.Z(this, _.OX, 2);
			}
			setValue(a) {
				return _.ln(this, _.OX, 2, a);
			}
			xc() {
				return _.sn(this, _.OX, 2);
			}
		};
		_.DFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			zq() {
				return _.mj(this, _.tY, 2, _.oj());
			}
			getValue() {
				return _.Z(this, _.sY, 3);
			}
			setValue(a) {
				return _.ln(this, _.sY, 3, a);
			}
			xc() {
				return _.sn(this, _.sY, 3);
			}
			tB() {
				return _.mj(this, _.uY, 4, _.oj());
			}
			Wy() {
				return _.Pm(this, 6);
			}
		};
		_.DFc.prototype.Zr = _.ba(200);
		_.EFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getName() {
				return _.l(this, 1);
			}
			getValue() {
				return _.Z(this, _.RX, 2);
			}
			setValue(a) {
				return _.ln(this, _.RX, 2, a);
			}
			xc() {
				return _.sn(this, _.RX, 2);
			}
			tB() {
				return _.mj(this, _.uY, 3, _.oj());
			}
			Mw() {
				return _.Pm(this, 4);
			}
			xC(a) {
				return _.Mj(this, 4, a);
			}
			Wy() {
				return _.Pm(this, 5);
			}
		};
		_.EFc.prototype.Zr = _.ba(199);
		_.FFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			zq() {
				return _.mj(this, _.tY, 1, _.oj());
			}
		};
		_.GFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Xs(this, 1);
			}
			setValue(a) {
				return _.In(this, 1, _.Eb(a));
			}
			xc() {
				return _.Fb(_.tn(this, 1)) != null;
			}
			Sb() {
				return _.Z(this, _.tY, 2);
			}
			Fe() {
				return _.sn(this, _.tY, 2);
			}
		};
		HFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 1);
			}
			Fe() {
				return _.sn(this, _.tY, 1);
			}
		};
		_.IFc = function(a) {
			return _.fj(a, HFc, 1, _.yY);
		};
		_.zY = function(a) {
			return _.fj(a, _.GFc, 2, _.yY);
		};
		_.JFc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		KFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			zq() {
				return _.mj(this, _.tY, 1, _.oj());
			}
			mE() {
				return _.Lm(this, 3);
			}
		};
		LFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 2);
			}
			Fe() {
				return _.sn(this, _.tY, 2);
			}
		};
		MFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 1);
			}
			Fe() {
				return _.sn(this, _.tY, 1);
			}
		};
		_.NFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 2);
			}
			Fe() {
				return _.sn(this, _.tY, 2);
			}
		};
		_.OFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getValue() {
				return _.Z(this, _.lY, 1);
			}
			setValue(a) {
				return _.ln(this, _.lY, 1, a);
			}
			xc() {
				return _.sn(this, _.lY, 1);
			}
			tB() {
				return _.mj(this, _.uY, 2, _.oj());
			}
			Wy() {
				return _.Pm(this, 3);
			}
		};
		_.OFc.prototype.Zr = _.ba(198);
		_.PFc = function(a, b) {
			return _.ln(a, _.tY, 2, b);
		};
		_.QFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			ig() {
				return _.Lm(this, 1);
			}
			Sb() {
				return _.Z(this, _.tY, 2);
			}
			Fe() {
				return _.sn(this, _.tY, 2);
			}
			mE() {
				return _.Lm(this, 3);
			}
			Wy() {
				return _.Pm(this, 4);
			}
			zo() {
				return _.mj(this, _.uY, 7, _.oj());
			}
			getSize() {
				return _.Lm(this, 8);
			}
		};
		_.RFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			QL() {
				return _.mj(this, _.tY, 1, _.oj());
			}
			mE() {
				return _.Lm(this, 2);
			}
		};
		FDc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getAction() {
				return _.Z(this, _.uY, 1);
			}
			hasAction() {
				return _.sn(this, _.uY, 1);
			}
		};
		_.AY = function(a) {
			return _.zc(a, 6, WEc);
		};
		_.BY = function(a) {
			return _.Z(a, yEc, 9);
		};
		_.SFc = function(a) {
			return _.Z(a, qFc, 1019);
		};
		_.TFc = function(a) {
			return _.fj(a, _.RFc, 1e3, _.CY);
		};
		_.DY = function(a) {
			return _.fj(a, _.CFc, 1017, _.CY);
		};
		_.tY = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
			JY() {
				return _.jj(this, _.CY);
			}
			getId() {
				return _.l(this, 1);
			}
			qm() {
				return _.zn(this, 1);
			}
			lja() {
				return _.Pm(this, 5);
			}
			uJa(a, b) {
				return _.Os(this, 1020, FDc, a, b);
			}
			OL() {
				return _.fj(this, _.QFc, 1001, _.CY);
			}
			oR() {
				return _.fj(this, MFc, 1004, _.CY);
			}
			Al() {
				return _.fj(this, PEc, 1005, _.CY);
			}
			hasImage() {
				return _.Dr(this, PEc, 1005, _.CY);
			}
			Bl() {
				return _.fj(this, LFc, 1021, _.CY);
			}
			hasLabel() {
				return _.Dr(this, LFc, 1021, _.CY);
			}
			N7() {
				return _.fj(this, KFc, 1003, _.CY);
			}
			yja() {
				return _.fj(this, AFc, 1009, _.CY);
			}
			S7() {
				return _.fj(this, _.xFc, 1011, _.CY);
			}
			getText() {
				return _.fj(this, _.uFc, 1002, _.CY);
			}
			setText(a) {
				return _.Ap(this, 1002, _.CY, a);
			}
			kR() {
				return _.fj(this, WDc, 1012, _.CY);
			}
		};
		_.tY.prototype.Mja = _.ba(202);
		_.tY.prototype.Wja = _.ba(168);
		_.EY = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 1);
			}
			Fe() {
				return _.sn(this, _.tY, 1);
			}
		};
		UFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 1);
			}
			Fe() {
				return _.sn(this, _.tY, 1);
			}
		};
		_.VFc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.Z(this, _.tY, 1);
			}
			Fe() {
				return _.sn(this, _.tY, 1);
			}
		};
		_.WFc = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
			Sb() {
				return _.Z(this, _.tY, 1);
			}
			Fe() {
				return _.sn(this, _.tY, 1);
			}
		};
		_.XFc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.GY = function(a) {
			return _.fj(a, SEc, 7, _.FY);
		};
		_.HY = function(a) {
			return _.Dr(a, SEc, 7, _.FY);
		};
		_.IY = function(a) {
			return _.Dr(a, _.rFc, 9, _.FY);
		};
		_.uY = class extends _.h {
			constructor(a) {
				super(a, 500);
			}
		};
		_.yY = [
			1,
			2,
			3,
			4
		];
		_.CY = [
			1e3,
			1001,
			1002,
			1003,
			1004,
			1005,
			1006,
			1007,
			1009,
			1010,
			1011,
			1012,
			1013,
			1014,
			1015,
			1016,
			1017,
			1018,
			1021,
			1022,
			1023,
			1025,
			1026
		];
		_.FY = [
			1,
			2,
			3,
			4,
			5,
			6,
			7,
			8,
			9,
			10,
			11,
			12,
			1e3,
			1001,
			1002,
			1003,
			1004,
			1005
		];
		var YFc = [
			0,
			() => JY,
			_.Jt,
			() => JY
		];
		var ZFc = [
			0,
			pY,
			_.Rt,
			() => KY,
			() => JY,
			-1,
			_.zt,
			_.Gt,
			[
				0,
				_.Et,
				_.Ut
			],
			_.zt,
			OEc
		];
		var $Fc = [
			0,
			2,
			_.zt,
			_.Rt,
			() => JY,
			3,
			_.Rt,
			() => KY
		];
		var aGc = [
			0,
			_.Rt,
			() => JY
		];
		var bGc = [
			0,
			_.Rt,
			() => JY
		];
		var cGc = [
			0,
			_.Gt,
			() => JY,
			_.Et,
			-1
		];
		var dGc = [
			0,
			_.Rt,
			() => JY,
			1,
			() => JY,
			-1
		];
		var eGc = [
			0,
			_.Rt,
			() => KY,
			_.zt
		];
		var fGc = [
			0,
			() => JY,
			1,
			() => eGc
		];
		var gGc = [
			0,
			() => JY,
			mFc,
			_.zt
		];
		var hGc = [
			0,
			1,
			_.Rt,
			() => JY,
			nFc,
			_.Rt,
			() => KY,
			1,
			_.Gt,
			OEc
		];
		var iGc = [
			0,
			_.zt,
			oEc,
			_.Rt,
			() => KY,
			_.Gt,
			-1,
			OEc,
			pY
		];
		var jGc = [
			0,
			_.Rt,
			() => JY
		];
		var kGc = [
			0,
			_.Tt,
			() => JY
		];
		var lGc = [0, () => JY];
		var mGc = [
			0,
			_.yY,
			_.St,
			() => lGc,
			_.St,
			() => kGc,
			_.St,
			[0],
			_.St,
			REc
		];
		var nGc = [
			0,
			_.Rt,
			() => JY,
			1,
			_.Ut
		];
		var oGc = [
			0,
			_.zt,
			() => JY
		];
		var pGc = [
			0,
			() => JY,
			_.Ut
		];
		var qGc = [
			0,
			1,
			() => JY
		];
		var rGc = [
			0,
			pY,
			_.Rt,
			() => KY,
			_.Gt,
			OEc
		];
		var sGc = [
			0,
			_.Ut,
			() => JY,
			_.Ut,
			_.Gt,
			2,
			_.Rt,
			() => KY,
			_.Ut,
			_.zt
		];
		var tGc = [
			0,
			_.Rt,
			() => JY,
			_.Vt
		];
		var uGc = [
			0,
			() => KY,
			2,
			_.Gt,
			-3
		];
		var JY = [
			-500,
			_.CY,
			_.zt,
			_.yt,
			kY,
			-2,
			_.Gt,
			_.yt,
			_.At(XEc),
			[
				0,
				_.Tt,
				_.zt,
				_.Gt,
				-6
			],
			_.Rt,
			() => KY,
			zEc,
			_.zt,
			989,
			_.St,
			() => tGc,
			_.St,
			() => sGc,
			_.St,
			() => $Fc,
			_.St,
			() => nGc,
			_.St,
			() => pGc,
			_.St,
			QEc,
			_.St,
			() => YFc,
			_.St,
			EEc,
			1,
			_.St,
			() => dGc,
			_.St,
			() => cGc,
			_.St,
			() => bGc,
			_.St,
			hEc,
			_.St,
			() => qGc,
			_.St,
			() => aGc,
			_.St,
			() => fGc,
			_.St,
			() => hGc,
			_.St,
			() => gGc,
			_.St,
			() => jGc,
			[
				0,
				_.zt,
				-1
			],
			_.Rt,
			() => uGc,
			_.St,
			() => oGc,
			_.St,
			() => rGc,
			_.St,
			() => iGc,
			1,
			_.St,
			() => ZFc,
			_.St,
			() => mGc
		];
		var vGc = [0, () => JY];
		var wGc = [0, () => JY];
		var xGc = [
			0,
			() => JY,
			_.zt,
			2,
			_.Ut,
			-1
		];
		var yGc = [
			-500,
			() => JY,
			10,
			_.Gt,
			987,
			_.Gt,
			-1
		];
		var zGc = [
			0,
			qY,
			_.Rt,
			() => KY,
			-1
		];
		var KY = [
			-500,
			_.FY,
			_.St,
			GEc,
			_.St,
			() => yGc,
			_.St,
			[
				0,
				_.zt,
				-1,
				1,
				aFc,
				_.Rt,
				[
					0,
					[
						1,
						3,
						4,
						5,
						6,
						7,
						8,
						9,
						10,
						11,
						12
					],
					_.St,
					[
						0,
						[
							0,
							_.yt,
							kY
						],
						_.zt,
						_.Gt,
						_.zt,
						_.Et
					],
					[
						0,
						_.Jt,
						-1,
						_.Ft,
						_.Ht
					],
					_.St,
					[0, _.Gt],
					_.St,
					[
						0,
						_.Jt,
						-3
					],
					_.St,
					[
						0,
						1,
						_.zt,
						[0, _.Gt],
						[
							0,
							_.Jt,
							_.du,
							-1,
							_.yt,
							kY,
							_.Jt,
							[
								0,
								_.Jt,
								1,
								_.Rt,
								[
									0,
									[
										10,
										11,
										12,
										13,
										14
									],
									1,
									_.Jt,
									3,
									_.xQa,
									1,
									jY,
									-1,
									_.St,
									[0, _.Jt],
									_.St,
									[0, _.Jt],
									_.St,
									[0, _.Jt],
									_.St,
									[0, jY],
									_.St,
									[0, jY],
									15,
									_.Jt
								],
								1,
								_.yt,
								_.At(iFc)
							],
							[
								0,
								1,
								_.Jt,
								1,
								_.Rt,
								iFc,
								3,
								_.du,
								-1,
								1,
								_.Jt,
								jY,
								-1
							],
							_.Vt,
							_.Jt,
							[
								0,
								_.Ht,
								-1,
								_.yt,
								kY,
								_.Jt,
								_.Ht,
								-1,
								1,
								_.yt,
								kY,
								aEc,
								_.yt,
								kY,
								[
									0,
									_.Ht,
									-1
								],
								_.Jt
							],
							1,
							[
								0,
								_.Rt,
								[
									0,
									6,
									_.Jt,
									jY,
									1,
									jY
								],
								1,
								_.Rt,
								[
									0,
									_.Vt,
									_.Jt,
									-1,
									jY,
									_.Jt,
									[
										0,
										_.Jt,
										-1
									],
									_.Jt,
									_.Rt,
									hFc,
									jY,
									-1
								],
								[
									0,
									jY,
									_.yt,
									kY,
									_.yt,
									_.At(hFc)
								]
							],
							[
								0,
								_.yt,
								_.At([
									0,
									_.Jt,
									-1
								])
							],
							_.Jt,
							TDc,
							_.yt,
							_.At([
								0,
								_.Jt,
								_.Rt,
								[
									0,
									_.du,
									-1
								],
								1,
								_.Jt,
								_.Vt,
								-1,
								3,
								_.Jt,
								_.ev,
								3,
								_.Rt,
								TDc,
								_.yt,
								kY,
								-1,
								jY,
								-1,
								_.Jt,
								YDc,
								_.Vt,
								-1,
								_.Rt,
								aEc,
								_.Ht,
								_.du
							]),
							_.Ft,
							_.yt,
							_.At(kFc),
							_.Jt,
							_.yt,
							_.At([
								0,
								hFc,
								jFc,
								jY
							]),
							_.yt,
							_.At(kFc),
							_.Ft,
							_.Jt
						],
						_.Gt,
						-2
					],
					_.St,
					[0],
					_.St,
					[
						0,
						_.zt,
						-2,
						_.Rt,
						lFc,
						[
							0,
							_.zt,
							-2
						],
						[
							0,
							_.Jt,
							[
								0,
								_.Jt,
								-6
							],
							[
								0,
								_.Jt,
								-5
							]
						],
						_.Rt,
						lFc
					],
					_.St,
					[0, _.zt],
					_.St,
					[
						0,
						_.Jt,
						_.Vt,
						-1,
						[
							0,
							_.Jt,
							-1,
							_.du,
							-1,
							_.Jt,
							-1,
							_.Ht,
							_.Jt,
							-1,
							_.Ht,
							_.jv
						],
						[
							0,
							_.Ht,
							_.Jt
						]
					],
					_.St,
					[
						0,
						[
							5,
							6,
							7
						],
						_.zt,
						[
							0,
							_.zt,
							[
								0,
								[8, 9],
								_.Ft,
								-6,
								_.St,
								_.jv,
								_.St,
								[
									0,
									_.Jt,
									-1
								]
							],
							_.zt
						],
						_.yt,
						_.At(eFc),
						_.Jt,
						_.St,
						[
							0,
							_.yt,
							_.At(eFc),
							_.Jt
						],
						_.St,
						[
							0,
							_.zt,
							-2
						],
						_.St,
						[0]
					],
					_.St,
					[0, _.Vt],
					_.St,
					[
						0,
						[
							0,
							_.Jt,
							-1,
							[
								0,
								_.Rt,
								fFc
							],
							[
								0,
								_.Rt,
								fFc
							]
						],
						[
							0,
							_.Jt,
							-1,
							bEc,
							_.Jt,
							-1,
							_.du,
							_.Jt,
							-1,
							_.du,
							_.Jt,
							[
								0,
								_.xQa,
								-3,
								_.Jt
							],
							_.Rt,
							gFc,
							_.du
						],
						_.Ht,
						[0, _.Ht]
					]
				],
				[
					0,
					_.zt,
					_.Jt,
					-1
				],
				_.yt,
				kY,
				_.zt,
				_.Rt,
				[
					0,
					rY,
					_.Kt,
					_.St,
					[
						0,
						[1, 2],
						_.Kt,
						_.EQa,
						_.Jt,
						-1
					],
					_.St,
					[0, _.ev],
					_.ev
				],
				_.Ut
			],
			_.St,
			BEc,
			_.St,
			[
				0,
				2,
				nFc
			],
			_.St,
			[
				-500,
				_.zt,
				xEc,
				_.zt,
				_.yt,
				_.At(nFc),
				_.yt,
				_.At([0, _.zt]),
				_.zt,
				993,
				[0, _.jv]
			],
			_.St,
			TEc,
			_.St,
			() => xGc,
			_.St,
			[
				0,
				_.zt,
				-1,
				_.yt,
				kY
			],
			_.St,
			JEc,
			_.St,
			VEc,
			_.St,
			() => vGc,
			987,
			_.St,
			() => wGc,
			_.St,
			[0],
			_.St,
			() => zGc,
			_.St,
			NEc,
			_.St,
			ZEc,
			_.St,
			LEc
		];
		var AGc = function(a) {
			clearTimeout(a.A);
			a.A = setTimeout(() => {
				for (let b of a.callbacks) b.F0a = false, b.VLa && b.fn(...b.args);
				a.callbacks = [];
				a.A = 0;
			}, a.interval);
		};
		var BGc = class {
			constructor() {
				this.interval = 85;
				this.A = 0;
				this.callbacks = [];
			}
			ED(a, b = false) {
				var c = {
					fn: a,
					qs: b,
					VLa: false
				};
				this.callbacks.push(c);
				return (...d) => {
					if (c.qs && !c.F0a) {
						c.F0a = true, c.VLa = false, a(...d);
					} else {
						c.args = d, c.VLa = true;
					}
					if (this.callbacks.indexOf(c) === -1) {
						this.callbacks.push(c);
					}
					AGc(this);
				};
			}
		};
		var CGc = new _.he("UseResizeMonitorIframeFallback");
		var DGc = class {
			get width() {
				throw Error("Ng");
			}
			get height() {
				throw Error("Og");
			}
		};
		var EGc = new DGc();
		var FGc = class {
			constructor() {
				this.listeners = [];
				this.H = this.I = 0;
			}
		};
		var LY = function(a) {
			if (a = a._cfc_resize_monitor_) return a;
		};
		var IGc = function(a, b) {
			var c = new BGc();
			return ECc((d) => {
				HGc(a, b, c).listeners.push({
					bU: d,
					vYa: "width"
				});
				return { bU: d };
			}, (d, e) => {
				d = e.bU;
				e = LY(b);
				for (let f = 0; f < e.listeners.length; f++) if (e.listeners[f].bU === d) {
					e.listeners.splice(f, 1);
					break;
				}
				d = LY(b);
				if (!d.listeners.length) {
					d.child && (d.child.contentWindow && delete d.child.contentWindow.onresize, delete d.child.onload, b.removeChild(d.child)), delete b._cfc_resize_monitor_, d.A && d.A.disconnect(), d.F && d.F.unobserve(b);
				}
			}).pipe(FCc(a.yb));
		};
		var HGc = function(a, b, c) {
			var d = LY(b);
			if (!d) {
				d = new FGc();
				b._cfc_resize_monitor_ = d;
				let e = c.ED((f) => {
					a.OAa(b, f);
				}, false);
				d.qz = e;
				if (ResizeObserver && !a.F) {
					d.F = a.A, a.A.observe(b), e();
				} else {
					c = JGc(e), b.appendChild(c), d.child = c, (b.ownerDocument.compareDocumentPosition(b) & 16) != 16 && (d.A = a.H.create(() => {
						if ((b.ownerDocument.compareDocumentPosition(b) & 16) == 16) {
							e();
							let f;
							if (!((f = d.A) == null)) {
								f.disconnect();
							}
						}
					}), d.A.observe(b, {
						attributes: true,
						subtree: true
					}));
				}
			}
			return d;
		};
		var MY = class {
			constructor() {
				this.yb = _.m(_.th);
				this.H = _.m(_.NA);
				this.F = _.m(CGc, { optional: true });
				if (ResizeObserver && !this.F) {
					this.A = new ResizeObserver((a) => {
						GGc(this, a);
					});
				}
			}
			OAa(a, b) {
				var c = LY(a);
				if (c) {
					var d = false;
					for (var e of c.listeners) e.vYa !== "none" && (d = true);
					e = {
						width: undefined,
						height: undefined
					};
					if (d) {
						b ? e = {
							width: b.width,
							height: b.height
						} : c.child ? (a = c.child.getBoundingClientRect(), e = {
							width: a.width,
							height: a.height
						}) : e = {
							width: a.clientWidth,
							height: a.clientHeight
						};
					}
					for (let f of c.listeners) switch (f.vYa) {
						case "both":
							f.bU(e);
							break;
						case "width":
							c.I !== e.width && f.bU(e);
							break;
						case "height":
							c.H !== e.height && f.bU(e);
							break;
						case "none": f.bU(EGc);
					}
					c.I = e.width;
					c.H = e.height;
				}
			}
		};
		MY.J = function(a) {
			return new (a || MY)();
		};
		MY.sa = _.Cd({
			token: MY,
			factory: MY.J,
			wa: "root"
		});
		new _.RF("dd/MMM/yyyy:HH:mm:ss Z", _.Cmb);
		var KGc = {
			de: RDc,
			de_DE: RDc,
			en_GB: {
				Tl: ".",
				Jm: ",",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤#,##0.00",
				nn: "GBP"
			},
			en_US: _.YQa,
			es: {
				Tl: ",",
				Jm: ".",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0\xA0%",
				Xo: "#,##0.00\xA0¤",
				nn: "EUR"
			},
			es_419: {
				Tl: ".",
				Jm: ",",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤#,##0.00",
				nn: "MXN"
			},
			fr: SDc,
			fr_FR: SDc,
			id: {
				Tl: ",",
				Jm: ".",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤#,##0.00",
				nn: "IDR"
			},
			it: {
				Tl: ",",
				Jm: ".",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "#,##0.00\xA0¤",
				nn: "EUR"
			},
			ja: {
				Tl: ".",
				Jm: ",",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤#,##0.00",
				nn: "JPY"
			},
			ko: {
				Tl: ".",
				Jm: ",",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤#,##0.00",
				nn: "KRW"
			},
			nl: {
				Tl: ",",
				Jm: ".",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤\xA0#,##0.00;¤\xA0-#,##0.00",
				nn: "EUR"
			},
			pl: {
				Tl: ",",
				Jm: "\xA0",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "#,##0.00\xA0¤",
				nn: "PLN"
			},
			pt_BR: {
				Tl: ",",
				Jm: ".",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤\xA0#,##0.00",
				nn: "BRL"
			},
			ru: {
				Tl: ",",
				Jm: "\xA0",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "нe\xA0чиcлo",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0\xA0%",
				Xo: "#,##0.00\xA0¤",
				nn: "RUB"
			},
			th: {
				Tl: ".",
				Jm: ",",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤#,##0.00",
				nn: "THB"
			},
			tr: {
				Tl: ",",
				Jm: ".",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "%#,##0",
				Xo: "¤#,##0.00",
				nn: "TRY"
			},
			zh_CN: {
				Tl: ".",
				Jm: ",",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "NaN",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤#,##0.00",
				nn: "CNY"
			},
			zh_TW: {
				Tl: ".",
				Jm: ",",
				oo: "%",
				un: "0",
				bp: "+",
				rn: "-",
				jo: "E",
				po: "‰",
				qn: "∞",
				Zo: "非數值",
				io: "#,##0.###",
				ep: "#E0",
				ap: "#,##0%",
				Xo: "¤#,##0.00",
				nn: "TWD"
			}
		};
		var LGc = [/^(.*\.)?stackdriver\.com$/];
		var NY = class extends Window {
			get H() {
				throw Error("vf");
			}
			get F() {
				throw Error("vf");
			}
		};
		NY.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(NY)))(b || NY);
			};
		})();
		NY.sa = _.Cd({
			token: NY,
			factory: function(a) {
				var b = null;
				if (a) {
					b = new (a || NY)();
				} else {
					b = _.aP() ? parent : window;
				}
				return b;
			},
			wa: "root"
		});
		var OY = class {
			constructor(a) {
				this.A = a;
			}
		};
		OY.J = function(a) {
			return new (a || OY)(_.ae(NY));
		};
		OY.sa = _.Cd({
			token: OY,
			factory: OY.J,
			wa: "root"
		});
		var NGc = Object.freeze("authuser autoresume b cloudshell cloudshellsafemode consoleReturnUrl consoleUI csesidx duet chat ce e eap facet_url facet_experiment_ids folder hl inv invt jsmode journey_id liveReload localDevAppName localDevModuleSetIds localDevPort localDevServer mjsmode memorystore mods organizationId p2env p2jsmode p2snapshot pantheon_testId quickstart reportTiming requestReason showFTMessage storageKey src token trial tutorial use_staging walkthrough_id walkthrough_tutorial_id embeddedAppsEnvironment".split(" "));
		Object.freeze(NGc.concat([
			"orgonly",
			"project",
			"supportedpurview"
		]));
		Object.freeze(NGc.concat(["orgonly", "supportedpurview"]));
		Object.freeze(["debugUI"]);
		Object.freeze({
			API_SECTION: [
				"endpoint-id",
				"endpointId",
				"returnUrl"
			],
			APP_DESIGN_CENTER_SECTION: ["space", "sduiData"],
			APPENGINE_SECTION: "serviceId versionId filename contentType size unit op startDate endDate sortCol sortDir".split(" "),
			AVERE_SECTION: ["endpoint-id", "endpointId"],
			BILLING_SECTION: [],
			CRASH_SECTION: [],
			COMPUTE_SECTION: "osPolicy osFamily isRunning osVersion cveId patchAvailable patchSeverity complianceState".split(" "),
			DEPLOYMENT_MANAGER_SECTION: ["preview"],
			LAUNCHER_SECTION: "solution endpointId endpoint-id env preview returnUrl".split(" "),
			MARKETPLACE_SECTION: "solution endpointId endpoint-id env preview returnUrl".split(" "),
			LOGS_SECTION: ["serviceId", "versionId"],
			DATASTORE_SECTION: "ns namespace kind filter gql sortCol sortDir queryType".split(" "),
			FOLDER_CREATE_SECTION: ["previousPage"],
			FREE_TRIAL_SECTION: [],
			HOME_SECTION: [],
			PARTNER_PORTAL_SECTION: [
				"env",
				"cloudBiEnv",
				"partnerId"
			],
			PROJECT_CREATE_SECTION: ["previousPage"],
			PROJECT_SECTION: [],
			IAM_ADMIN_SECTION: [],
			SECURITY_SECTION: ["location"],
			STORAGE_SECTION: ["userProject"],
			SUPPORT_SECTION: ["caseId", "accountId"],
			PRODUCER_PORTAL: ["partnerAccountId"],
			TELECOM_NETWORK_AUTOMATION_SECTION: ["oc"],
			VERTEX_AI_SECTION: ["vertex_ai_region"],
			rif_ucp_hub: ["service", "env"]
		});
		var OGc = function(a, b) {
			if (!a.F.has(b)) {
				a.F.set(b, Promise.withResolvers());
			}
			return a.F.get(b);
		};
		var HCc = class {
			constructor() {
				this.F = new Map();
				this.A = new Map();
			}
			getServiceConnectionInfo(a) {
				return OGc(this, a).promise;
			}
			markServiceLoaded(a, b) {
				if (this.A.has(a)) {
					let { info: c, stack: d } = this.A.get(a);
					throw Error("Pg`" + a + "`" + c.registryName + "`" + !!c.frame + "`" + b.registryName + "`" + !!b.frame, { cause: d });
				}
				this.A.set(a, {
					info: b,
					stack: Error("Qg")
				});
				OGc(this, a).resolve(b);
			}
			markServiceLoadFailed(a, b) {
				OGc(this, a).reject(b);
			}
		};
		var PGc = new _.he("XChannelConnectionInfo", { factory: function() {
			return ICc();
		} });
		var QGc = new _.hP("45762915");
		var RGc = class extends _.Rj {};
		var TGc = class extends _.Oj {
			constructor(a) {
				super(SGc);
				this.data = a;
			}
		};
		var SGc = new _.Uv("o");
		var UGc = class extends RGc {
			constructor(a, b) {
				super();
				this.channelName = a;
				this.channel = b;
			}
			send(a) {
				this.channel.send(a);
			}
		};
		var NCc = class extends UGc {
			constructor(a, b, c) {
				super(a, cEc({
					channelName: a,
					destination: b,
					origin: c,
					onMessage: (d) => {
						this.dispatchEvent(new TGc(d.data));
					}
				}));
			}
		};
		var MCc = class extends UGc {
			constructor(a, b, c, d) {
				super(a, _.Ytb({
					channelName: a,
					destination: b,
					origin: c,
					iframe: d,
					onMessage: (e) => {
						this.dispatchEvent(new TGc(e.data));
					}
				}));
			}
		};
		var VGc = function(a, b) {
			return _.In(a, 1, _.Eb(b));
		};
		var PY = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var QY = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var WGc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var RY = function(a, b) {
			return _.ht(a, 1, b);
		};
		var SY = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var TY = function(a, b) {
			return _.ln(a, SY, 1, b);
		};
		var UY = class extends _.h {
			constructor(a) {
				super(a);
			}
			Tf() {
				return _.Z(this, SY, 1);
			}
		};
		var XGc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var VY = function(a, b) {
			return _.In(a, 1, _.Eb(b));
		};
		var YGc = function(a, b) {
			return _.ln(a, PY, 2, b);
		};
		var $Gc = function() {
			var a = new XY();
			var b = new WGc();
			return _.Ap(a, 4, WY, b);
		};
		var YY = function(a, b) {
			return _.ln(a, UY, 6, b);
		};
		var XY = class extends _.h {
			constructor(a) {
				super(a);
			}
			getData() {
				return _.Z(this, QY, 5);
			}
			ze() {
				return _.sn(this, QY, 5);
			}
		};
		var WY = [3, 4];
		var aHc = [
			0,
			WY,
			_.Tt,
			[0, _.Tt],
			_.St,
			[0],
			_.St,
			[0],
			[
				0,
				_.zt,
				_.DQa
			],
			[
				0,
				[
					0,
					_.Ct,
					_.zt
				],
				-1
			]
		];
		var bHc = _.Xc(XY, aHc);
		var cHc = _.Yc(aHc);
		var dHc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getData() {
				return _.l(this, 2);
			}
			ze() {
				return _.zn(this, 2);
			}
		};
		var eHc = function(a, b) {
			return _.Ym(a, 1, b);
		};
		var fHc = function(a, b) {
			return _.cn(a, 2, b);
		};
		var gHc = function(a, b) {
			return _.Ym(a, 3, b);
		};
		var hHc = function(a, b) {
			return _.Ym(a, 4, b);
		};
		var iHc = function(a, b) {
			return _.Ym(a, 5, b);
		};
		var jHc = function(a, b) {
			return _.Zm(a, 6, b);
		};
		var kHc = function(a, b) {
			return _.Zm(a, 7, b);
		};
		var lHc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		KX.prototype.getKey = function() {
			return this.A;
		};
		KX.prototype.getValue = function() {
			return this.F;
		};
		KX.prototype.clone = function() {
			return new KX(this.A, this.F);
		};
		var mHc = class {
			constructor(a) {
				this.A = [];
				if (a) a: {
					let c;
					if (a instanceof mHc) {
						if (c = a.hE(), a = a.getValues(), this.Dn() <= 0) {
							var b = this.A;
							for (let d = 0; d < c.length; d++) b.push(new KX(c[d], a[d]));
							break a;
						}
					} else {
						c = _.jqa(a);
						a = _.iqa(a);
					}
					for (b = 0; b < c.length; b++) this.insert(c[b], a[b]);
				}
			}
			insert(a, b) {
				var c = this.A;
				c.push(new KX(a, b));
				a = c.length - 1;
				b = this.A;
				for (c = b[a]; a > 0;) {
					let d = a - 1 >> 1;
					if (b[d].getKey() > c.getKey()) {
						b[a] = b[d];
						a = d;
					} else break;
				}
				b[a] = c;
			}
			remove() {
				var a = this.A;
				var b = a.length;
				var c = a[0];
				if (!(b <= 0)) {
					if (b == 1) a.length = 0;
					else {
						a[0] = a.pop();
						a = 0;
						b = this.A;
						let e = b.length;
						let f = b[a];
						for (; a < e >> 1;) {
							var d = (a << 1) + 1;
							let g = (a << 1) + 2;
							d = g < e && b[g].getKey() < b[d].getKey() ? g : d;
							if (b[d].getKey() > f.getKey()) break;
							b[a] = b[d];
							a = d;
						}
						b[a] = f;
					}
					return c.getValue();
				}
			}
			m7a() {
				var a = this.A;
				if (a.length != 0) return a[0].getValue();
			}
			getValues() {
				var a = this.A;
				var b = [];
				var c = a.length;
				for (let d = 0; d < c; d++) b.push(a[d].getValue());
				return b;
			}
			hE() {
				var a = this.A;
				var b = [];
				var c = a.length;
				for (let d = 0; d < c; d++) b.push(a[d].getKey());
				return b;
			}
			clone() {
				return new mHc(this);
			}
			Dn() {
				return this.A.length;
			}
			isEmpty() {
				return this.A.length === 0;
			}
			clear() {
				this.A.length = 0;
			}
		};
		var nHc = class extends mHc {
			enqueue(a, b) {
				this.insert(a, b);
			}
		};
		var oHc = function(a) {
			if (a.A.length === 0) {
				a.A = a.F, a.A.reverse(), a.F = [];
			}
		};
		_.aa = JCc.prototype;
		_.aa.enqueue = function(a) {
			this.F.push(a);
		};
		_.aa.m7a = function() {
			oHc(this);
			var a = this.A;
			return a[a.length - 1];
		};
		_.aa.Dn = function() {
			return this.A.length + this.F.length;
		};
		_.aa.isEmpty = function() {
			return this.A.length === 0 && this.F.length === 0;
		};
		_.aa.clear = function() {
			this.A = [];
			this.F = [];
		};
		_.aa.contains = function(a) {
			return _.Ba(this.A, a) || _.Ba(this.F, a);
		};
		_.aa.remove = function(a) {
			{
				var b = this.A;
				let c = Array.prototype.lastIndexOf.call(b, a, b.length - 1);
				if (c >= 0) {
					Array.prototype.splice.call(b, c, 1), b = true;
				} else {
					b = false;
				}
			}
			return b || _.Ca(this.F, a);
		};
		_.aa.getValues = function() {
			var a = [];
			for (var b = this.A.length - 1; b >= 0; --b) a.push(this.A[b]);
			b = this.F.length;
			for (let c = 0; c < b; ++c) a.push(this.F[c]);
			return a;
		};
		var ZY = function(a) {
			if (a.U) {
				a.dispatchEvent(new pHc(qHc(a)));
			}
		};
		var rHc = function(a, ...b) {
			for (let c of b) a.I.enqueue(c);
			ZY(a);
		};
		var sHc = function(a, b) {
			b = b(a.fa);
			ZY(a);
			return b;
		};
		var qHc = function(a) {
			return kHc(jHc(iHc(hHc(gHc(fHc(eHc(new lHc(), a.port), a.getState()), a.H), a.A), a.F), a.I.getValues()), a.aa.getValues().map((b) => {
				var c = new dHc();
				c = _.Ym(c, 1, b.Foa);
				b = _.Lj(c, 2, b.data);
				return _.oc(b);
			}));
		};
		var tHc = class extends _.Rj {
			constructor() {
				super(...arguments);
				this.Fa = null;
				this.U = false;
				this.I = new JCc();
				this.fa = new nHc();
				this.R = new Map();
				this.ta = this.oa = this.na = 0;
				this.za = new Set();
			}
			get Na() {
				return this.I.Dn();
			}
			get aa() {
				return this.fa;
			}
			get H() {
				return this.na;
			}
			set H(a) {
				this.na = a;
				ZY(this);
			}
			get A() {
				return this.oa;
			}
			set A(a) {
				this.oa = a;
				ZY(this);
			}
			get F() {
				return this.ta;
			}
			set F(a) {
				this.ta = a;
				ZY(this);
			}
			Yf() {
				ZY(this);
			}
		};
		var vHc = class extends _.Oj {
			constructor(a) {
				super(uHc);
				this.data = a;
			}
		};
		var uHc = new _.Uv("p");
		var xHc = class extends _.Oj {
			constructor() {
				super(wHc);
			}
		};
		var wHc = new _.Uv("q");
		var zHc = class extends _.Oj {
			constructor() {
				super(yHc);
			}
		};
		var yHc = new _.Uv("r");
		var pHc = class extends _.Oj {
			constructor(a) {
				super(AHc);
				this.snapshot = a;
			}
		};
		var AHc = new _.Uv("s");
		var BHc = class extends _.Rj {
			constructor() {
				super(...arguments);
				this.H = false;
			}
			I(a) {
				this.H = a;
			}
		};
		var DHc = class extends _.Oj {
			constructor(a) {
				super(CHc);
				this.data = a;
			}
		};
		var CHc = new _.Uv("t");
		var FHc = class extends _.Oj {
			constructor(a) {
				super(EHc);
				this.socket = a;
			}
		};
		var EHc = new _.Uv("u");
		var HHc = class extends _.Oj {
			constructor(a) {
				super(GHc);
				this.state = a;
			}
		};
		var GHc = new _.Uv("v");
		var IHc = class {
			constructor(a, b) {
				this.F = a;
				this.A = b;
			}
			serializeMessage(a) {
				try {
					return this.F.serializeMessage(a);
				} catch (b) {
					return this.A.serializeMessage(a);
				}
			}
			deserializeMessage(a) {
				try {
					return this.F.deserializeMessage(a);
				} catch (b) {
					return this.A.deserializeMessage(a);
				}
			}
		};
		var JHc = class {
			constructor(a) {
				this.U8a = a;
			}
			serializeMessage(a) {
				return a.serialize();
			}
			deserializeMessage(a) {
				return _.ad(this.U8a, a);
			}
		};
		var KHc = class {
			constructor(a, b) {
				this.A = a;
				this.F = b;
			}
			serializeMessage(a) {
				return _.Ja(this.A(a));
			}
			deserializeMessage(a) {
				a = _.Paa(a);
				return this.F(a);
			}
		};
		var LHc = class {
			create(a, b, c, d = {}) {
				var e = d.Wn;
				a = new JHc(a);
				b = new KHc(b, c);
				switch (d.Moa) {
					case 1: return new IHc(b, a);
					case 0: return new IHc(a, b);
					case 2:
						if (!e) throw Error("Rg");
						return new IHc(e, a);
					default: return new IHc(b, a);
				}
			}
		};
		var MHc = class {
			constructor() {
				this.A = new Set();
				this.logger = null;
			}
			release(a) {
				this.A.delete(a);
			}
		};
		var NHc = new Set([0]);
		var OHc = 1e5 - NHc.size;
		var PHc = function(a, b) {
			return _.Lj(a, 1, b);
		};
		var QHc = function(a, b) {
			return _.Zm(a, 2, b);
		};
		var RHc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var SHc = function(a) {
			var b = new zHc();
			_.x(function* () {
				yield Promise.resolve().then(() => {
					a.dispatchEvent(b);
				});
			});
		};
		var THc = function(a, b) {
			a.X.push(b);
		};
		var UHc = function(a) {
			a.Aa = true;
			a.X.forEach((b) => {
				b();
			});
		};
		var WHc = class extends tHc {
			constructor(a, b) {
				super();
				this.Ea = a;
				this.state = 1;
				this.X = [];
				this.Aa = false;
				_.ck(this, () => {
					this.X = [];
				});
				_.ck(this, () => {});
				if (typeof b === "number") this.port = b;
				else {
					this.port = _.yj(b, 1);
					this.state = _.Lm(b, 2);
					this.na = _.yj(b, 3);
					this.oa = _.yj(b, 4);
					this.ta = _.yj(b, 5);
					a = _.Yp(b, XY, 6).map((c) => c.clone());
					for (let c of a) this.I.enqueue(c);
					_.Yp(b, dHc, 7).forEach((c) => {
						this.fa.enqueue(_.yj(c, 1), {
							Foa: _.yj(c, 1),
							data: c.getData()
						});
					});
				}
			}
			send(a) {
				this.Ea.send(this, a);
			}
			close() {
				this.Ea.close(this);
			}
			getState() {
				return this.state;
			}
			Yf(a) {
				this.state = a;
				if (!this.Aa && VHc.has(this.state)) {
					UHc(this);
				}
				if (this.state === 1) {
					this.dispatchEvent(new xHc()), this.dispose();
				} else {
					this.state === 5 ? SHc(this) : this.state === 9 && this.close();
				}
				super.Yf(a);
			}
		};
		var VHc = new Set([
			6,
			7,
			10,
			11,
			8,
			9,
			1
		]);
		var XHc = function(a, b) {
			a.sockets.delete(b.port);
			b.Yf(1);
		};
		var aZ = function(a, b) {
			for (; b.A < b.F + a.eeb && b.Na > 0;) {
				var c = b;
				if (c.I.isEmpty()) c = undefined;
				else {
					var d = c.I;
					oHc(d);
					d = d.A.pop();
					ZY(c);
					c = d;
				}
				if (!c) break;
				VY(c, b.A++);
				$Y(a, b, c);
			}
		};
		var ZHc = function(a, b) {
			var c = new WHc(a, b);
			c.U = a.H;
			_.ck(c, () => {
				setTimeout(() => {
					a.ports.release(c.port);
				}, 0);
			});
			var d = () => {
				if (a.H) {
					a.dispatchEvent(new HHc(YHc(a)));
				}
			};
			a.eventHandler.listen(c, AHc, d);
			_.ck(a, () => {
				a.eventHandler.Xz(c, AHc, d);
			});
			a.sockets.set(c.port, c);
			return c;
		};
		var $Hc = function(a) {
			var b = ZHc(a, 0);
			b.Yf(2);
			a.dispatchEvent(new FHc(b));
		};
		var $Y = function(a, b, c, d = false) {
			var e = (f = 0) => {
				a.eF(f);
				var g = _.Xs(c, 1);
				if (f >= a.maxRetries) {
					aIc(b, g);
					XHc(a, b);
				} else {
					if (!d) {
						let k = setTimeout(() => {
							e(f + 1);
						}, a.yIa);
						b.R.set(g, {
							Hs: k,
							Y8b: f
						});
					}
					a.R.send(a.Wn.serializeMessage(c));
				}
			};
			b.za.add(_.Xs(c, 1));
			e();
		};
		var bIc = function(a) {
			a: {
				var b = a.ports;
				if (b.A.size >= OHc) throw Error("Sg");
				for (;;) {
					var c = Math.floor(Math.random() * 100001);
					if (!NHc.has(c) && !b.A.has(c)) {
						b.A.add(c);
						b = c;
						break a;
					}
				}
				b = undefined;
			}
			b = ZHc(a, b);
			a.A = a.IBa();
			b.F = a.A;
			b.A = a.A + 1;
			b.Yf(3);
			c = YY(ZGc(VY(new XY(), a.A)), TY(new UY(), RY(new SY(), b.port)));
			$Y(a, b, c);
			return b;
		};
		var YHc = function(a) {
			return QHc(PHc(new RHc(), a.channelName), Array.from(a.sockets.values(), (b) => qHc(b)));
		};
		var aIc = function(a, b) {
			if (a.R.has(b)) {
				var c = a.R.get(b).Hs;
				clearTimeout(c);
				a.R.delete(b);
			}
		};
		var cIc = function(a, b) {
			a.za.delete(b);
			aIc(a, b);
			for (b === a.F && a.F++; !a.za.has(a.F) && a.F < a.A;) a.F++;
		};
		var dIc = function(a, b) {
			for (var c = b.getState() === 5; !b.aa.isEmpty() && b.aa.m7a().Foa === b.H && c;) {
				let d = sHc(b, (e) => e.remove());
				if (d.Foa !== b.H) throw Error("$g");
				b.dispatchEvent(new vHc(d.data));
				a.dispatchEvent(new DHc(d.data));
				b.H++;
			}
		};
		var eIc = function(a, b) {
			b.Fa = setTimeout(() => {
				XHc(a, b);
			}, a.yIa * a.maxRetries);
		};
		var fIc = function(a, b, c) {
			switch (b.getState()) {
				case 3:
				case 4:
					if (c !== a.A) break;
					cIc(b, c);
					b.Yf(5);
					break;
				case 5:
				case 6:
				case 7:
				case 9:
					cIc(b, c);
					aZ(a, b);
					b.getState() === 6 && c === b.A - 1 && b.Yf(7);
					break;
				case 8:
					cIc(b, c);
					c === b.A - 1 && (b.Yf(10), eIc(a, b));
					break;
				case 11: cIc(b, c), c === b.A - 1 && XHc(a, b);
			}
		};
		var gIc = function(a, b, c, d) {
			if (b.getState() === 5) {
				if (b.H <= c) {
					sHc(b, (f) => undefined);
				}
				var e = YY(YGc(VY(new XY(), b.A - 1), VGc(new PY(), c)), TY(new UY(), RY(new SY(), b.port)));
				$Y(a, b, e, true);
				dIc(a, b);
			}
		};
		var OCc = class extends BHc {
			constructor(a, { channelName: b = "__NOT_PROVIDED__SHOULD_NOT_READ__", ports: c = new MHc(), parameters: d = {}, Wn: e = new LHc().create(XY, cHc, bHc) } = {}) {
				super();
				this.R = a;
				this.logger = null;
				this.A = this.F = 0;
				this.eventHandler = new _.ok(this);
				this.sockets = new Map();
				this.ports = c;
				this.Wn = e;
				this.channelName = b;
				var f;
				this.IBa = (f = d.IBa) != null ? f : LCc;
				var g;
				this.eeb = (g = d.eeb) != null ? g : 10;
				var k;
				this.maxRetries = (k = d.maxRetries) != null ? k : 3;
				var p;
				this.yIa = (p = d.yIa) != null ? p : 200;
				var r;
				this.eF = (r = d.eF) != null ? r : KCc;
				_.ck(this, () => {
					this.sockets.forEach((v) => {
						v.close();
					});
					this.sockets.clear();
				});
				_.bk(this, this.R);
				_.bk(this, this.eventHandler);
				this.eventHandler.listen(a, SGc, (v) => {
					v = v.data;
					try {
						var w = this.Wn.deserializeMessage(v);
					} catch (N) {
						return;
					}
					v = 0;
					var D = _.Z(w, UY, 6).Tf();
					D = _.Ys(D, 1);
					D = _.Gk(D);
					if (this.sockets.has(D)) {
						v = D;
					}
					if ((v = this.sockets.get(v)) && _.Fb(_.tn(w, 1)) != null) {
						D = _.Xs(w, 1);
						var G;
						if (G = _.sn(w, PY, 2)) {
							G = _.Z(w, PY, 2);
							G = _.Fb(_.tn(G, 1)) != null;
						}
						if (G) {
							var L = _.Z(w, PY, 2);
							L = _.Xs(L, 1);
						}
						if (_.Dr(w, XGc, 3, WY)) switch (D = L, L = _.Xs(w, 1), v.getState()) {
							case 2:
								if (D) break;
								v.Yf(4);
								w = _.Z(w, UY, 6).Tf();
								w = _.Ys(w, 1);
								w = _.Gk(w);
								v.port = w;
								this.sockets.delete(0);
								this.sockets.set(v.port, v);
								$Hc(this);
								this.A = this.IBa();
								v.F = this.A;
								v.A = this.A + 1;
								this.F = L;
								v.H = this.F + 1;
								w = YY(YGc(ZGc(VY(new XY(), this.A)), VGc(new PY(), L)), TY(new UY(), RY(new SY(), v.port)));
								$Y(this, v, w);
								break;
							case 3: if (D != null && D === this.A) {
								fIc(this, v, D);
								if (v.getState() !== 5) throw Error("bh");
								v.A = v.A || this.A + 1;
								this.F = L;
								v.H = this.F + 1;
								w = YY(YGc(VY(new XY(), v.A - 1), VGc(new PY(), L)), TY(new UY(), RY(new SY(), v.port)));
								$Y(this, v, w, true);
								aZ(this, v);
								dIc(this, v);
							}
						}
						else {
							if (L != null) {
								fIc(this, v, L);
							}
							if (L = w.ze()) {
								L = w.getData();
								L = _.zn(L, 1);
							}
							if (L) {
								L = w.getData(), L = _.l(L, 1), gIc(this, v, D, L);
							}
							if (_.Dr(w, WGc, 4, WY) && !new Set([
								1,
								2,
								3
							]).has(v.getState())) switch (v.H = D + 1, w = YY(YGc(VY(new XY(), v.A - 1), VGc(new PY(), D)), TY(new UY(), RY(new SY(), v.port))), $Y(this, v, w, true), v.getState()) {
								case 4:
								case 5:
									v.Yf(9);
									break;
								case 6:
									v.Yf(8);
									break;
								case 7:
									v.Yf(10);
									eIc(this, v);
									break;
								case 9:
								case 8:
								case 11: break;
								case 10:
									eIc(this, v);
									break;
								default: throw Error("ah");
							}
						}
					}
				});
			}
			close(a) {
				switch (a.getState()) {
					case 1: throw Error("Tg");
					case 2:
					case 3:
						XHc(this, a);
						break;
					case 4:
					case 5:
						a.Yf(6);
						rHc(a, YY($Gc(), TY(new UY(), RY(new SY(), a.port))));
						aZ(this, a);
						break;
					case 8:
					case 6:
					case 7:
					case 10:
					case 11: throw Error("Ug`" + a.getState());
					case 9:
						a.Yf(11);
						rHc(a, YY($Gc(), TY(new UY(), RY(new SY(), a.port))));
						aZ(this, a);
						break;
					default: throw Error("Vg`" + a.getState());
				}
			}
			send(a, b) {
				switch (a.getState()) {
					case 1: throw Error("Wg");
					case 2: throw Error("Xg");
					case 3:
					case 4:
						var c = new XY();
						var d = new QY();
						d = _.Lj(d, 1, b);
						b = new TextEncoder().encode(b);
						b = _.In(d, 2, _.eb(b, false, true));
						b = _.ln(c, QY, 5, b);
						rHc(a, YY(b, TY(new UY(), RY(new SY(), a.port))));
						break;
					case 5:
					case 9:
						c = new XY();
						d = new QY();
						d = _.Lj(d, 1, b);
						b = new TextEncoder().encode(b);
						b = _.In(d, 2, _.eb(b, false, true));
						b = _.ln(c, QY, 5, b);
						rHc(a, YY(b, TY(new UY(), RY(new SY(), a.port))));
						aZ(this, a);
						break;
					case 6:
					case 7:
					case 8:
					case 11:
					case 10: throw Error("Yg`" + a.getState());
					default: throw Error("Zg`" + a.getState());
				}
			}
		};
		var hIc = function(a) {
			if (a.isDisposed()) throw Error("dh`" + a.name);
		};
		var QCc = class extends RGc {
			constructor(a) {
				super();
				this.name = a;
				this.logger = null;
				this.A = new Set();
				this.F = false;
				_.ck(this, () => {
					new Set(this.A).forEach(this.disconnectXappChannel.bind(this));
				});
			}
			send(a) {
				hIc(this);
				try {
					this.F = true;
					for (let b of this.A) b.onXappMessage(a);
				} finally {
					this.F = false;
				}
			}
			onXappMessage(a) {
				hIc(this);
				this.dispatchEvent(new TGc(a));
			}
			connectXappChannel(a) {
				hIc(this);
				if (this.F) throw Error("eh`connect");
				if (!this.A.has(a)) {
					this.A.add(a), a.connectXappChannel(this);
				}
			}
			disconnectXappChannel(a) {
				if (this.F) throw Error("eh`disconnect");
				if (this.A.has(a)) {
					this.A.delete(a), a.disconnectXappChannel(this);
				}
			}
		};
		var iIc = function(a, b) {
			return _.Lj(a, 1, b);
		};
		var bZ = class extends _.h {
			constructor(a) {
				super(a);
			}
			Pc() {
				return _.Z(this, _.lw, 3);
			}
		};
		var kIc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getData() {
				return _.l(this, 1);
			}
			ze() {
				return _.zn(this, 1);
			}
		};
		var mIc = function() {
			var a = new lIc();
			return _.cn(a, 1, 1);
		};
		var lIc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getError() {
				return _.Lm(this, 1);
			}
			hasError() {
				return _.wn(this, 1) != null;
			}
			kd() {
				return _.l(this, 2);
			}
		};
		var dZ = function(a) {
			return _.Dr(a, lIc, 5, cZ);
		};
		var nIc = function(a, b) {
			return _.Ym(a, 6, b);
		};
		var eZ = class extends _.h {
			constructor(a) {
				super(a);
			}
			getHeaders() {
				return _.fj(this, bZ, 3, cZ);
			}
			xba(a) {
				return _.Ap(this, 3, cZ, a);
			}
			getData() {
				return _.fj(this, kIc, 4, cZ);
			}
			ze() {
				return _.Dr(this, kIc, 4, cZ);
			}
			getIdentifier() {
				return _.yj(this, 6);
			}
		};
		var cZ = [
			3,
			4,
			5
		];
		var oIc = [
			0,
			cZ,
			1,
			_.Gt,
			_.St,
			[
				0,
				_.zt,
				1,
				TDc,
				_.zt
			],
			_.St,
			[
				0,
				_.zt,
				_.DQa
			],
			_.St,
			[
				0,
				_.Ut,
				_.zt
			],
			_.Et
		];
		var pIc = _.Xc(eZ, oIc);
		var qIc = _.Yc(oIc);
		var IDc = class extends Error {
			constructor() {
				super("Timeout must be a positive non-zero integer.");
				this.name = "ValidationError";
				Object.setPrototypeOf(this, new.target.prototype);
			}
		};
		var rIc = class extends Error {
			constructor() {
				super("Failed to establish connection.");
				this.name = "ConnectionTimeoutError";
				Object.setPrototypeOf(this, new.target.prototype);
			}
		};
		var sIc = class extends Error {
			constructor() {
				super("Socket closed.");
				this.name = "ConnectionClosedError";
				Object.setPrototypeOf(this, new.target.prototype);
			}
		};
		var MDc = class extends Error {
			constructor() {
				super("Failed to receive response within the specified timeout.");
				this.name = "RequestTimeoutError";
				Object.setPrototypeOf(this, new.target.prototype);
			}
		};
		var fZ = class extends Error {
			constructor(a) {
				super(a);
				this.name = "StreamError";
				Object.setPrototypeOf(this, new.target.prototype);
			}
		};
		var tIc = class extends Error {
			constructor(a) {
				super(`Code: ${a.Ff()}, message: ${a.getMessage()}`);
				this.status = a;
				this.name = "StatusError";
				Object.setPrototypeOf(this, new.target.prototype);
			}
		};
		var uIc = function(a, b) {
			return _.Ym(a, 1, b);
		};
		var vIc = function(a, b) {
			return _.Ym(a, 2, b);
		};
		var wIc = function(a, b) {
			return _.Lj(a, 3, b);
		};
		var xIc = class extends _.h {
			constructor(a) {
				super(a);
			}
			Uk() {
				return _.l(this, 3);
			}
		};
		var yIc = function(a, b) {
			return _.Zm(a, 1, b);
		};
		var zIc = function(a, b) {
			return _.Zm(a, 2, b);
		};
		var AIc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var BIc = new class {
			PHa() {}
			removeStream() {}
		}();
		var CIc = class {};
		var LX = function() {
			_.x(function* () {});
		};
		var DIc = class extends CIc {
			constructor() {
				super(...arguments);
			}
		};
		new DIc();
		var EIc = function(a, b) {
			if (!b.ze() || a.state === "CLOSED_REMOTE" || a.Tc()) {
				nIc(b, a.getIdentifier());
				switch (a.getState()) {
					case "IDLE":
						if (!_.Dr(b, bZ, 3, cZ)) {
							a.reportError(new fZ("Headers must be sent before any other frames."));
							return;
						}
						break;
					case "OPEN":
						_.vn(b, 2) ? a.Yf("CLOSED_LOCAL") : dZ(b) && a.Yf("CLOSED");
						break;
					case "CLOSED_LOCAL":
						if (dZ(b)) a.Yf("CLOSED");
						else {
							a.reportError(new fZ("A stream that is in the CLOSED_LOCAL state can only send a StopStream frame."));
							return;
						}
						break;
					case "CLOSED_REMOTE":
						(_.vn(b, 2) || dZ(b)) && a.Yf("CLOSED");
						break;
					case "CLOSED":
						a.reportError(new fZ("A stream that is in the CLOSED state cannot send frames."));
						return;
					default: return;
				}
				b = a.F.serializeMessage(b);
				try {
					a.socket.send(b);
				} catch (c) {
					if (c instanceof Error) {
						a.reportError(c);
					} else {
						typeof c === "string" ? a.reportError(Error(c)) : a.reportError(Error("gh"));
					}
				}
			} else a.reportError(new fZ(`Data cannot be sent because the stream is in ${a.getState()} state.`));
		};
		var FIc = function(a, b, c) {
			if (a.callbacks.has(b)) {
				a.callbacks.get(b).forEach((d) => {
					d(c);
				});
			}
		};
		var GIc = function(a, b) {
			if (_.dn(b, 6)) if (_.dn(b, 6) && b.getIdentifier() === 0) a.reportError(new fZ("0 is a reserved stream ID."));
			else if (b.getIdentifier() !== a.getIdentifier()) a.reportError(new fZ("Received frame with identifier that does not match this stream."));
			else switch (_.Dr(b, bZ, 3, cZ) ? a.g8a(b.getHeaders()) : b.ze() && a.NHa(b.getData(), a.state === "CLOSED_LOCAL" || a.Tc(), a.getState()), a.getState()) {
				case "IDLE":
					_.Dr(b, bZ, 3, cZ) || a.reportError(new fZ("Headers must be received before any other frames."));
					break;
				case "OPEN":
					_.vn(b, 2) ? a.Yf("CLOSED_REMOTE") : dZ(b) && (a.Yf("CLOSED"), a.dispose());
					break;
				case "CLOSED_LOCAL":
					if (_.vn(b, 2) || dZ(b)) {
						a.Yf("CLOSED");
						a.dispose();
					}
					break;
				case "CLOSED_REMOTE":
					dZ(b) ? (a.Yf("CLOSED"), a.dispose()) : a.reportError(new fZ("A stream that is in the CLOSED_REMOTE state can only receive a StopStream frame."));
					break;
				case "CLOSED": a.reportError(new fZ("A stream that is in the CLOSED state cannot receive frames."));
			}
			else a.reportError(new fZ("Frames must be associated with a stream."));
		};
		var HIc = class extends _.Rj {
			constructor(a, b, c) {
				super();
				this.socket = a;
				this.identifier = b;
				this.F = c;
				this.logger = null;
				this.state = "IDLE";
				this.callbacks = new Map();
				this.JX = null;
				this.A = new Map([
					["CLOSED", []],
					["CLOSED_LOCAL", ["CLOSED"]],
					["CLOSED_REMOTE", ["CLOSED"]],
					["OPEN", [
						"CLOSED_LOCAL",
						"CLOSED_REMOTE",
						"CLOSED"
					]],
					["IDLE", ["OPEN"]]
				]);
				this.e$ = new _.gw();
				BIc.PHa(this);
				this.on("STATE", (d) => {
					if (d === "CLOSED") {
						Promise.resolve().then(() => {
							this.dispose();
						});
					}
				});
				THc(this.socket, () => {
					this.reportError(new sIc());
					this.Yf("CLOSED");
				});
			}
			Ce() {
				this.callbacks.clear();
				BIc.removeStream(this);
				this.e$.resolve();
				super.Ce();
			}
			on(a, b) {
				var c;
				var d = (c = this.callbacks.get(a)) != null ? c : [];
				d.push(b);
				this.callbacks.set(a, d);
				return this;
			}
			sendHeaders(a, b) {
				a = new eZ().xba(a);
				b = _.Mj(a, 2, b);
				EIc(this, b);
			}
			emit(a, b) {
				if (this.JX) {
					var c = new eZ();
					b = _.Mj(c, 2, b);
					a = this.JX.serializeMessage(a);
					c = new kIc();
					c = _.Lj(c, 1, a);
					TCc(a.length, this.e$.promise);
					_.Ap(b, 4, cZ, c);
					EIc(this, b);
				} else this.reportError(new fZ("Data serializer was not provided."));
			}
			close(a = mIc()) {
				var b = new eZ();
				a = _.Ap(b, 5, cZ, a);
				EIc(this, a);
			}
			getState() {
				return this.state;
			}
			reportError(a) {
				try {
					FIc(this, "ERROR", a);
				} catch (b) {
					this.dispose();
				}
			}
			getIdentifier() {
				return this.identifier;
			}
			removeListener(a, b) {
				var c;
				a = (c = this.callbacks.get(a)) != null ? c : [];
				b = a.indexOf(b);
				if (b >= 0) {
					a.splice(b, 1);
				}
				return this;
			}
			Yf(a) {
				var b = this.state;
				if (this.A.has(b)) {
					this.A.get(b).includes(a) ? b = true : (this.reportError(new fZ(`Invalid state transition: ${b} -> ${a}`)), b = false);
				} else {
					this.reportError(new fZ(`Stream is in unsupported state: ${b}`)), b = false;
				}
				if (b) {
					this.state = a, FIc(this, "STATE", a);
				}
			}
			Tc() {
				return this.state === "OPEN";
			}
			NHa(a, b, c) {
				if (b) {
					this.JX ? (TCc(a.getData().length, this.e$.promise), a = this.JX.deserializeMessage(a.getData()), FIc(this, "DATA", a)) : this.reportError(new fZ("Data serializer was not provided."));
				} else {
					this.reportError(new fZ(`Data cannot be received because the stream is in ${c} state.`));
				}
			}
		};
		var IIc = /^[\$_\d\w]+\.[\$_\d\w]+$/;
		var JIc = function(a, b) {
			a.JX = b;
			a.gya.resolve();
		};
		var KIc = function(a, b) {
			a.V0a.push(b);
		};
		var gZ = function(a, b) {
			var c = a.sendHeaders;
			var d = new bZ();
			b = _.ln(d, _.lw, 3, b);
			c.call(a, b, true);
			a.close();
		};
		var LIc = class extends HIc {
			constructor() {
				super(...arguments);
				this.gya = new _.gw();
				this.OXa = this.gya.promise;
				this.V0a = [];
			}
			Ce() {
				super.Ce();
				this.gya.resolve();
			}
			NHa(a, b, c) {
				var d = this;
				var e = () => super.NHa;
				return _.x(function* () {
					yield d.OXa;
					e().call(d, a, b, c);
				});
			}
			emit(a, b) {
				var c = this;
				var d = () => super.emit;
				return _.x(function* () {
					yield c.OXa;
					d().call(c, a, b);
				});
			}
			close(a) {
				var b = this;
				var c = () => super.close;
				return _.x(function* () {
					yield Promise.resolve().then(() => {
						c().call(b, a);
					});
				});
			}
			g8a(a) {
				this.Yf("OPEN");
				if (a.Pc()) gZ(this, iY(hY(3), "Status can only be received by client endpoints."));
				else if (_.l(a, 1)) {
					var b = _.l(a, 1);
					if (IIc.test(b)) for (let c of this.V0a) c(a);
					else gZ(this, iY(hY(3), "Malformed frame. Invalid service target."));
				} else gZ(this, iY(hY(3), "Malformed frame. Missing service target."));
			}
		};
		var MIc = function(a) {
			return _.x(function* () {
				yield Promise.all(Array.from(a.streams.values()).map((b) => undefined));
				a.eventHandler.removeAll();
				a.state = "STOPPED";
				a.A.forEach((b) => {
					b.dispose();
				});
			});
		};
		var QIc = function(a, b) {
			var c = (e) => {
				var f = e.socket;
				f.U = a.F;
				var g = (k) => {
					if (_.dn(k, 6)) {
						var p = k.getHeaders();
						if (p && !a.H.some((r) => _.yj(r, 1) === k.getIdentifier() && _.yj(r, 2) === f.port && r.Uk() === _.l(p, 1))) {
							a.H.push(wIc(vIc(uIc(new xIc(), k.getIdentifier()), f.port), _.l(p, 1))), a.F && a.dispatchEvent(new NIc(OIc(a)));
						}
						GIc(PIc(a, k.getIdentifier(), f), k);
					}
				};
				a.eventHandler.listen(f, uHc, (k) => undefined);
				e = a.Taa.filter((k) => _.yj(k, 2) === f.port);
				for (let k of e) g(nIc(new eZ(), _.yj(k, 1)).xba(iIc(new bZ(), k.Uk())));
			};
			a.eventHandler.listen(b, EHc, c);
			var d = () => {
				if (a.F) {
					a.dispatchEvent(new NIc(OIc(a)));
				}
			};
			a.eventHandler.listen(b, GHc, d);
			_.ck(b, () => {
				a.eventHandler.Xz(b, EHc, c);
				a.eventHandler.Xz(b, GHc, d);
			});
			_.bk(a, b);
			$Hc(b);
		};
		var PIc = function(a, b, c) {
			var d = `${c.port}-${b}`;
			if (!a.streams.has(d)) {
				let e = new LIc(c, b, a.R);
				a.streams.set(d, e);
				THc(c, () => {
					e.close();
				});
				e.on("STATE", (f) => {
					if (f === "CLOSED") {
						a.streams.delete(d);
					}
				});
				KIc(e, (f) => {
					var g = _.l(f, 1);
					var [k, p] = g.split(".");
					if (g = a.U.get(k)) {
						g = RIc(a, k, g), SIc(a, g, k, p, e);
					} else {
						gZ(e, iY(hY(5), `Service not found: ${k}.`));
					}
					f = {
						nJa: _.l(f, 1),
						id: _.l(f, 4)
					};
					e.e$.resolve(f);
				});
			}
			return a.streams.get(d);
		};
		var SIc = function(a, b, c, d, e) {
			_.x(function* () {
				var f = () => _.x(function* () {
					return (yield b).getMethod(d);
				});
				e.on("DATA", TIc(a, f, e));
				if (a.Taa.some((g) => _.yj(g, 1) === e.getIdentifier() && g.Uk() === `${c}.${d}`)) {
					let g;
					let k;
					yield (k = (g = yield b).restore) == null ? undefined : k.call(g, e);
				}
				{
					let g = (yield f()).getSerializer({
						Moa: a.options.Moa,
						Wn: a.options.Wn
					});
					JIc(e, g);
				}
			});
		};
		var TIc = function(a, b, c) {
			return (d) => _.x(function* () {
				try {
					let e = yield b();
					yield a.invoke(e, d, c);
				} catch (e) {
					if (e instanceof tIc) {
						gZ(c, e.status);
					} else {
						gZ(c, iY(hY(13), e instanceof Error ? e.message : "Internal error"));
					}
				}
			});
		};
		var OIc = function(a) {
			return yIc(zIc(new AIc(), Array.from(a.A.values(), (b) => YHc(b))), a.H);
		};
		var UIc = class extends _.Rj {
			constructor(a, b, c = {}, d = false) {
				super();
				this.U = b;
				this.options = c;
				this.F = d;
				this.state = "IDLE";
				this.streams = new Map();
				this.I = new Map();
				this.eventHandler = new _.ok(this);
				this.logger = null;
				this.A = new Set();
				a.forEach((f) => {
					if (typeof f.I === "function") {
						f.I(this.F);
					}
					this.A.add(f);
					if (this.state === "STARTED") {
						QIc(this, f);
					}
				});
				this.R = new LHc().create(eZ, qIc, pIc);
				var e;
				this.Taa = (e = c.Taa) != null ? e : [];
				this.H = [...this.Taa];
				_.bk(this, this.eventHandler);
			}
			Ce() {
				MIc(this).then(() => undefined);
			}
			start() {
				if (this.isDisposed()) throw Error("hh");
				if (this.state !== "IDLE") throw Error("ih");
				for (let a of this.A) QIc(this, a);
				if (globalThis.window) {
					this.eventHandler.listen(globalThis.window, "pagehide", (a) => {
						if (!a.persisted) {
							this.dispose();
						}
					});
				}
				this.state = "STARTED";
			}
			getState() {
				return this.state;
			}
			invoke(a, b, c) {
				return a.call(b, c);
			}
		};
		var NIc = class extends _.Oj {
			constructor(a) {
				super(VIc);
				this.snapshot = a;
			}
		};
		var VIc = new _.Uv("w");
		var VCc = new Set();
		var UCc = class {
			constructor(a) {
				this.create = a;
				this.map = new Map();
			}
			get(a) {
				if (!this.map.has(a)) {
					this.map.set(a, this.create(a));
				}
				return this.map.get(a);
			}
			clear() {
				this.map.clear();
			}
		};
		var WIc = WCc();
		var XIc = WCc();
		var YIc = function(a, b) {
			new _.ok(a).listen(a, VIc, (c) => {
				b.write(c.snapshot);
			});
			return a;
		};
		var ZIc = class {
			constructor() {
				this.services = new Map();
				this.A = new Set();
				this.options = {};
			}
			addService(a) {
				if (this.services.has(a.getIdentifier())) throw Error("jh");
				this.services.set(a.getIdentifier(), a);
				return this;
			}
			addLazyService(a, b) {
				if (a.getId() === "") throw Error("kh");
				if (this.services.has(a.getId())) throw Error("jh");
				this.services.set(a.getId(), b);
				return this;
			}
			withRestorationStrategy(a) {
				this.F = a;
				return this;
			}
			withTransport(a, b) {
				a = typeof a !== "string" ? a : WIc(b, a);
				if (this.A.has(a)) throw Error("lh");
				this.A.add(a);
				return this;
			}
			withOptions(a) {
				this.options = Object.assign({}, a);
				return this;
			}
			buildSync() {
				if (this.F) throw Error("mh");
				var a = new UIc(Array.from(this.A), new Map(this.services), this.options, false);
				this.clear();
				return a;
			}
			build() {
				var a = this;
				return _.x(function* () {
					if (!a.F) return a.buildSync();
					var b = a.F;
					var c = yield b.read();
					if (!c) return YIc(new UIc(Array.from(a.A), new Map(a.services), a.options, true), b);
					var d = Promise.all(c == null ? undefined : _.mj(c, RHc, 2, _.oj()).map((g) => b.e6b(g)));
					var e = new Set(c == null ? undefined : _.mj(c, RHc, 2, _.oj()).map((g) => _.l(g, 1)));
					var f = Array.from(a.A).filter((g) => !e.has(g.channelName));
					return YIc(new UIc([...f, ...yield d], new Map(a.services), Object.assign({}, a.options, { Taa: _.Yp(c, xIc, 1) }), true), b);
				});
			}
			clear() {
				this.A = new Set();
				this.services.clear();
				this.F = undefined;
			}
		};
		var $Ic = null;
		var hZ = class {
			constructor() {
				new ZIc().withTransport(SCc());
				this.F = new Map();
				this.A = new Map();
				ICc();
				new Promise(() => {});
				this.registryName = null;
			}
			static get instance() {
				if (!$Ic) {
					$Ic = new hZ();
				}
				return $Ic;
			}
			kL(a, b = {}) {
				if ("serverWindow" in b) {
					var c;
					if ((c = b.channelName) == null) {
						let k;
						c = (k = frameElement) == null ? undefined : k.getAttribute("data-channel-name");
						if (!c) throw Error("oh");
					}
					var d;
					var e;
					var f = (e = this.F.get(c)) != null ? e : PCc(b.serverWindow, (d = b.origin) != null ? d : location.origin);
					this.F.set(c, f);
					return new a(c, f, b.options);
				}
				if ("serverFrame" in b) {
					if ((d = b.channelName) == null) {
						d = b.serverFrame, e = d.getAttribute("data-channel-name"), e || (e = `pantheon-xchannel-${(Date.now() + Math.random()).toString(36)}`, d.setAttribute("data-channel-name", e)), d = e;
					}
					let k;
					e = (k = (c = this.A.get(d)) == null ? undefined : c.get(b.serverFrame)) != null ? k : PCc(window, (f = b.origin) != null ? f : location.origin, b.serverFrame);
					if (!this.A.has(d)) {
						this.A.set(d, new WeakMap());
					}
					let p;
					if (!((p = this.A.get(d)) == null)) {
						p.set(b.serverFrame, e);
					}
					return new a(d, e, b.options);
				}
				var g;
				return new a(RCc((g = b.registry) != null ? g : "shell-xapp-service"), b.options);
			}
			get server() {
				return Promise.resolve(null);
			}
			listenToConnectionFromFrame() {
				throw Error("nh");
			}
		};
		hZ.J = function(a) {
			return new (a || hZ)();
		};
		hZ.sa = _.Cd({
			token: hZ,
			factory: () => hZ.instance,
			wa: "root"
		});
		var aJc = function(a, b) {
			return _.x(function* () {
				var c = yield a.F.getServiceConnectionInfo(b.serviceId.getId());
				if (c.window === window) return a.xchannelService.kL(b, { registry: c.registryName });
				if (c.frame) {
					if (window !== top) throw Error("ph");
					var d = c.listenToConnectionFromFrame(window);
					return a.xchannelService.kL(b, {
						serverFrame: c.frame,
						channelName: d
					});
				}
				if (Object.prototype.toString.call(frameElement) !== "[object HTMLIFrameElement]") throw Error("qh`" + frameElement);
				d = c.listenToConnectionFromFrame(frameElement);
				return a.xchannelService.kL(b, {
					serverWindow: c.window,
					channelName: d
				});
			});
		};
		var iZ = class {
			constructor() {
				this.F = _.m(PGc);
				this.xchannelService = _.m(hZ);
				this.A = new Map();
			}
			kL(a) {
				var b = this;
				return _.x(function* () {
					if (b.A.has(a)) return b.A.get(a);
					var c = aJc(b, a);
					b.A.set(a, c);
					return c;
				});
			}
		};
		iZ.J = function(a) {
			return new (a || iZ)();
		};
		iZ.sa = _.Cd({
			token: iZ,
			factory: iZ.J,
			wa: "root"
		});
		var jZ = class {
			constructor() {
				this.A = _.m(_.HC);
				this.Ga = _.m(_.Jf);
				this.xb = _.M();
				this.disabled = _.M(false);
				this.Pv = _.W(() => {
					var a;
					var b = (a = this.xb()) == null ? undefined : _.SFc(a);
					a = this.disabled();
					return b ? (a ? _.l(b, 2) || "Currently disabled." : b.getText()) || "" : "";
				});
				this.F = _.Kg((a) => {
					var b = this.Pv();
					this.A.message = b;
					if (this.disabled()) {
						this.Ga.nativeElement.classList.add("mat-mdc-tooltip-disabled"), a(() => {
							this.Ga.nativeElement.classList.remove("mat-mdc-tooltip-disabled");
						});
					}
					if (b) {
						this.Ga.nativeElement.classList.add("mat-mdc-tooltip-trigger"), a(() => {
							this.Ga.nativeElement.classList.remove("mat-mdc-tooltip-trigger");
						});
					}
				});
			}
			Rb() {
				this.A.Rb();
			}
			Ba() {
				this.F.destroy();
				this.A.Ba();
				this.Ga.nativeElement.classList.remove("mat-mdc-tooltip-trigger");
			}
		};
		jZ.J = function(a) {
			return new (a || jZ)();
		};
		jZ.Oa = _.We({
			type: jZ,
			features: [_.mh([_.HC])]
		});
		_.kZ = class {};
		_.lZ = class {
			constructor() {
				this.A = _.m(_.Jf);
			}
			get VUb() {
				return this.A.nativeElement;
			}
		};
		_.lZ.J = function(a) {
			return new (a || _.lZ)();
		};
		_.lZ.sa = _.Cd({
			token: _.lZ,
			factory: _.lZ.J
		});
		_.bJc = class {
			constructor(a, b) {
				this.name = a;
				this.id = b;
			}
		};
		var mZ = class {
			constructor() {
				this.F = this.A = new _.ml(false);
				var a = matchMedia("(prefers-reduced-motion: reduce)");
				_.Af(a, "change").pipe(_.Ak(), _.uf((b) => b.matches), _.bh(a.matches), _.Sg()).subscribe(this.A);
			}
			get en() {
				return this.A.value;
			}
		};
		mZ.J = function(a) {
			return new (a || mZ)();
		};
		mZ.sa = _.Cd({
			token: mZ,
			factory: mZ.J,
			wa: "root"
		});
		var nZ = class extends _.bJc {
			constructor() {
				super("prefersReducedMotion", "0-1");
				this.service = _.m(mZ);
			}
			F() {
				return _.YCc(this.service.en);
			}
			A() {
				return this.service.F.pipe(_.uf((a) => _.YCc(a)));
			}
		};
		nZ.J = function(a) {
			return new (a || nZ)();
		};
		nZ.sa = _.Cd({
			token: nZ,
			factory: nZ.J,
			wa: "root"
		});
		var oZ = class extends _.bJc {
			constructor() {
				super("viewerWidth", "0-2");
				this.H = _.m(MY);
			}
			F() {
				return aDc(-1);
			}
			A(a) {
				return IGc(this.H, a.VUb).pipe(_.uf((b) => aDc(b.width)));
			}
		};
		oZ.J = function(a) {
			return new (a || oZ)();
		};
		oZ.sa = _.Cd({
			token: oZ,
			factory: oZ.J,
			wa: "root"
		});
		_.cJc = function(a) {
			return new Map(a.signals.map((b) => {
				var c = b.name;
				var d = new _.sY();
				d = _.Lj(d, 1, b.id);
				return [c, d.setValue(b.F())];
			}));
		};
		_.dJc = class {
			constructor({ colorScheme: a }) {
				this.signals = [
					_.m(nZ),
					_.m(oZ),
					a
				].filter((b) => !!b);
			}
		};
		var gY = class extends Error {
			constructor(a) {
				super(a);
			}
		};
		var cY = class extends Error {
			constructor(a, b) {
				super(a, b);
			}
		};
		var eJc = class extends cY {
			constructor(a) {
				super(a);
			}
		};
		var eY = class extends cY {};
		var bY = class extends cY {
			constructor(a, ...b) {
				super(`Function "${a}" cannot be called with the given argument types: ${b.filter((c) => c !== undefined).map((c) => `(${c.type === "duration" || c.type === "timestamp" || typeof c.value === "string" ? `"${c.value}"` : c.value} : ${c.type})`).join(", ")}.`);
			}
		};
		var fJc = class {
			constructor(a) {
				this.index = a;
			}
			contains(a) {
				a = bDc(a);
				return a === undefined ? false : this.index.has(a);
			}
			get(a) {
				a = bDc(a);
				if (a !== undefined) {
					var b;
					return (b = this.index.get(a)) == null ? undefined : b[1];
				}
			}
			entries() {
				return cDc(this.index.entries(), ([, [a, b]]) => [a, b]);
			}
			get size() {
				return this.index.size;
			}
			merge(a) {
				return new fJc(new Map([...this.index.entries(), ...a.index.entries()]));
			}
		};
		var pZ = BigInt(1e9);
		var hJc = BigInt("-9223372036854775808");
		var iJc = BigInt("9223372036854775807");
		var jJc = BigInt(-62135596800);
		var kJc = BigInt(253402300799);
		var lJc = /^(?=.*\d[a-z])([-+])?(?:(\d+(?:\.\d+)?)h)?(?:(\d+(?:\.\d+)?)m)?(?:(\d+(?:\.\d+)?)s)?(?:(\d+(?:\.\d+)?)ms)?(?:(\d+(?:\.\d+)?)us)?(?:(\d+(?:\.\d+)?)ns)?$/;
		var dY = function(a) {
			return new mJc(a / pZ, Number(a % pZ));
		};
		var mJc = class {
			constructor(a, b) {
				this.A = a * pZ + BigInt(b);
				if (this.A < hJc || this.A > iJc) throw new cY("range");
			}
			get seconds() {
				return this.A / pZ;
			}
			get nanos() {
				return Number(this.A % pZ);
			}
			plus(a) {
				return dY(this.A + a.A);
			}
			minus(a) {
				return dY(this.A - a.A);
			}
			equals(a) {
				return a instanceof mJc && this.A === a.A;
			}
			toString() {
				if (this.A === BigInt(0)) return "0s";
				var a = this.A < BigInt(0);
				var b = a ? -this.A : this.A;
				var c = Number(b % pZ);
				a = (a ? "-" : "") + (b / pZ).toString();
				if (c !== 0) {
					b = c.toString().padStart(9, "0"), c % 1e6 === 0 ? b = b.slice(0, 3) : c % 1e3 === 0 && (b = b.slice(0, 6)), a += "." + b;
				}
				return a + "s";
			}
		};
		var nJc = class {
			constructor(a, b) {
				this.seconds = a;
				this.nanos = b;
				if (a < jJc || a > kJc) throw new cY("range");
				if (b < 0 || b >= 1e9) throw new cY("range");
			}
			plus(a) {
				var b = this.seconds + a.seconds;
				a = this.nanos + a.nanos;
				if (a >= 1e9) {
					b += BigInt(1), a -= 1e9;
				} else {
					if (a < 0) {
						b -= BigInt(1), a += 1e9;
					}
				}
				return new nJc(b, a);
			}
			minus(a) {
				return this.plus(dY(-a.A));
			}
			equals(a) {
				return a instanceof nJc && this.seconds === a.seconds && this.nanos === a.nanos;
			}
			toString() {
				var a = new Date(Number(this.seconds) * 1e3).toISOString().split(".")[0];
				if (this.nanos > 0) {
					a += "." + this.nanos.toString().padStart(9, "0").replace(/0+$/, "");
				}
				return a + "Z";
			}
		};
		var oJc = new Map([
			["bool", $X("bool")],
			["int", $X("int")],
			["uint", $X("uint")],
			["double", $X("double")],
			["string", $X("string")],
			["bytes", $X("bytes")],
			["list", $X("list")],
			["map", $X("map")],
			["null_type", $X("null_type")],
			["dyn", $X("dyn")],
			["type", $X("type")],
			["google.protobuf.Any", $X("google.protobuf.Any")],
			["google.protobuf.Timestamp", $X("google.protobuf.Timestamp")],
			["google.protobuf.Duration", $X("google.protobuf.Duration")]
		]);
		var pJc = /\s/g;
		var qJc = /\s/;
		var rJc = /\u2212/g;
		var sJc = (BigInt(1) << BigInt(63)) - BigInt(1);
		var tJc = -(BigInt(1) << BigInt(63));
		var uJc = class {
			constructor() {
				var a = KGc.en_US || _.YQa;
				this.A = a.Tl;
				var b = a.Jm;
				a = a.rn;
				if (b) {
					qJc.test(b) ? this.F = pJc : this.F = new RegExp(kDc(b), "g");
				}
				if (a && a !== "-" && a !== "−") {
					this.H = new RegExp(kDc(a), "g");
				}
			}
			parseInt(a) {
				var b = this.normalize(a);
				if (b === null) return null;
				var c = b.indexOf(".");
				a = c === -1 ? b : b.substring(0, c);
				b = c === -1 ? "" : b.substring(c + 1);
				if (/[^0]/.test(b)) return null;
				if (a === "" || a === "-") return /\d/.test(b) ? BigInt(0) : null;
				try {
					let d = BigInt(a);
					return d > sJc || d < tJc ? null : d;
				} catch (d) {
					return null;
				}
			}
			normalize(a) {
				if (a.trim() === "") return null;
				a = a.trim();
				a = a.replace(rJc, "-");
				if (this.H) {
					a = a.replace(this.H, "-");
				}
				if (this.F) {
					a = a.replace(this.F, "");
				}
				if (this.A && this.A !== ".") {
					a = a.replace(new RegExp(kDc(this.A), "g"), ".");
				}
				return a;
			}
		};
		var nDc = (BigInt(1) << BigInt(63)) - BigInt(1);
		var mDc = -(BigInt(1) << BigInt(63));
		var pDc = (BigInt(1) << BigInt(64)) - BigInt(1);
		var vJc = new Intl.NumberFormat("en-US", {
			useGrouping: true,
			maximumFractionDigits: 0
		});
		var wJc = new Intl.NumberFormat("en-US", {
			useGrouping: true,
			maximumFractionDigits: 17
		});
		var xJc = new uJc();
		var yJc = new Set([
			"_&&_",
			"_||_",
			"_?_:_",
			"@not_strictly_false"
		]);
		var AJc = class {
			constructor() {
				this.functions = new Map(zJc.map((a) => {
					var b;
					var c = a.name;
					var d = (b = a.Gn) != null ? b : false;
					return [`${c}:${d}:${a.ne}`, a.evaluate];
				}));
				this.A = new Set(zJc.map((a) => a.name));
			}
			dispatch(a, b, c) {
				var d = this.functions.get(`${a}:${b !== undefined}:${c.length}`);
				if (!d) {
					if (this.A.has(a)) throw new eY(`No such overload for function "${a}"`);
					throw new gY(`Unsupported function: "${a}" with ${b !== undefined ? "a target and " : ""}${c.length} argument(s).`);
				}
				if (!yJc.has(a)) {
					if ((b == null ? undefined : b.type) === "error") return b;
					for (let e of c) if (e.type === "error") return e;
				}
				return d(c, b);
			}
		};
		var BJc = class {
			constructor(a, b) {
				this.parent = a;
				this.A = b;
			}
			get(a) {
				if (this.A.has(a)) return this.A.get(a);
				if (this.parent instanceof BJc) return this.parent.get(a);
				if (this.parent.has(a)) return this.parent.get(a);
				if (oJc.has(a)) throw new gY("Type denotations are not supported.");
				if (a.includes(".")) throw new gY("Field selection and proto/enum lookup are not supported.");
				throw new eY(`Signal with name "${a}" was not found.`);
			}
		};
		var qZ = function(a, b, c, d) {
			if (_.Dr(b, _.mEc, 6, _.mY)) return CJc(a, _.fj(b, _.mEc, 6, _.mY), c, d);
			if (_.Dr(b, _.RX, 3, _.mY)) return EDc(_.fj(b, _.RX, 3, _.mY));
			if (_.Dr(b, _.lY, 4, _.mY)) {
				a = _.fj(b, _.lY, 4, _.mY).getName();
				c = c.get(a)();
				if (c.type === "error") throw c.value;
				return c;
			}
			if (_.Dr(b, _.lEc, 7, _.mY)) {
				a: {
					b = _.fj(b, _.lEc, 7, _.mY);
					var e = [];
					for (var f of _.mj(b, _.QX, 1, _.oj())) {
						b = qZ(a, f, c, d);
						if (b.type === "error") {
							c = b;
							break a;
						}
						e.push(b);
					}
					c = ZX(e);
				}
				return c;
			}
			if (_.Dr(b, _.kEc, 8, _.mY)) {
				a: {
					b = _.fj(b, _.kEc, 8, _.mY);
					if (_.l(b, 1) !== "") throw new gY("Message expressions are not supported.");
					f = [];
					for (e of _.mj(b, _.jEc, 2, _.oj())) {
						if (_.jj(e, _.nEc) !== 3) throw new eJc("Unexpected key kind for map entry.");
						b = qZ(a, _.fj(e, _.QX, 3, _.nEc), c, d);
						if (b.type === "error") {
							c = b;
							break a;
						}
						let g = qZ(a, e.getValue(), c, d);
						if (g.type === "error") {
							c = g;
							break a;
						}
						f.push([b, g]);
					}
					c = {
						type: "map",
						value: gJc(f)
					};
				}
				return c;
			}
			if (_.Dr(b, iEc, 9, _.mY)) return DJc(a, _.fj(b, iEc, 9, _.mY), c, d);
			throw new gY(`Expression type "${_.jj(b, _.mY)}" is not supported.`);
		};
		var CJc = function(a, b, c, d) {
			if (d && !yJc.has(b.getFunction())) {
				var e = b.Gn() ? qZ(a, b.Fn(), c, d) : undefined;
				var f = _.mj(b, _.QX, 3, _.oj()).map((g) => qZ(a, g, c, d));
				return a.A.dispatch(b.getFunction(), e, f);
			}
			if (b.Gn()) try {
				e = qZ(a, b.Fn(), c, false);
			} catch (g) {
				if (g instanceof cY) e = hDc(g);
				else throw g;
			}
			f = _.mj(b, _.QX, 3, _.oj()).map((g) => {
				try {
					return qZ(a, g, c, false);
				} catch (k) {
					if (k instanceof cY) return hDc(k);
					throw k;
				}
			});
			if (d) return a.A.dispatch(b.getFunction(), e, f);
			try {
				return a.A.dispatch(b.getFunction(), e, f);
			} catch (g) {
				if (g instanceof cY) return hDc(g);
				throw g;
			}
		};
		var DJc = function(a, b, c, d) {
			var e = qZ(a, _.Z(b, _.QX, 4), c, d);
			var f = qZ(a, _.Z(b, _.QX, 2), c, d);
			if (f.type === "list") var g = f.value.elements.map((k, p) => [_.UX(p), k]);
			else if (f.type === "map") g = f.value.entries();
			else throw new eJc("Iter range must be a list or map.");
			for (let [k, p] of g) {
				let r = k;
				let v = p;
				g = new Map([[_.l(b, 3), () => e]]);
				if (_.l(b, 8) === "" && f.type === "list") {
					g.set(_.l(b, 1), () => v);
				} else {
					g.set(_.l(b, 1), () => r), g.set(_.l(b, 8), () => v);
				}
				g = new BJc(c, g);
				let w = qZ(a, _.Z(b, _.QX, 5), g, false);
				if (w.type === "error") {
					e = w;
					break;
				}
				if (w.type === "bool" && !w.value) break;
				e = qZ(a, _.Z(b, _.QX, 6), g, false);
			}
			c = new BJc(c, new Map([[_.l(b, 3), () => e]]));
			return qZ(a, b.A(), c, d);
		};
		var EJc = class {
			constructor() {
				this.A = new AJc();
			}
			evaluate(a, b) {
				return qZ(this, a, new BJc(b, new Map()), true);
			}
		};
		_.FJc = function(a, b) {
			for (let [c, d] of b.entries()) {
				b = c;
				let e = d;
				if (a.signals.has(b)) continue;
				let f = e.getValue();
				a.signals.set(b, _.Yi(() => a.A.evaluate(f, a.signals)));
			}
		};
		_.rZ = function(a, b, c) {
			var d;
			if (!((d = a.signals.get(b)) == null)) {
				d.set(c);
			}
		};
		_.sZ = function(a, b) {
			return a.A.evaluate(b, a.signals);
		};
		_.tZ = class {
			constructor() {
				this.signals = new Map();
				this.A = new EJc();
				var a = _.m(_.dJc);
				_.FJc(this, new Map(Array.from(_.cJc(a).values(), (d) => [d.getId(), d.getValue()])));
				this.F = new Map(this.signals);
				var b = _.m(_.lZ);
				var c = _.m(_.ag);
				for (let d of a.signals) d.A(b).pipe(_.Ak(c)).subscribe((e) => {
					if (e = e.getValue()) {
						_.rZ(this, d.id, _.sZ(this, e));
					}
				});
			}
		};
		_.tZ.J = function(a) {
			return new (a || _.tZ)();
		};
		_.tZ.sa = _.Cd({
			token: _.tZ,
			factory: _.tZ.J
		});
		var uZ;
		uZ = function(a, b, c, d) {
			var e;
			return (b = (e = d.get(b)) == null ? undefined : e.getValue()) ? _.sZ(a.Jb, b).value : c;
		};
		_.vZ = class {
			constructor() {
				this.Jb = _.m(_.tZ);
			}
			evaluate(a, b, c, d) {
				var e;
				return (e = uZ(this, a, c, d)) != null ? e : c;
			}
		};
		_.vZ.J = function(a) {
			return new (a || _.vZ)();
		};
		_.vZ.sa = _.Cd({
			token: _.vZ,
			factory: _.vZ.J
		});
		var GJc = class {
			constructor(a) {
				this.A = a;
			}
			BMa(a) {
				this.A.BMa(a);
				for (let b of a.QL()) {
					if (!this.A.xd) break;
					this.kj(b);
				}
			}
			CMa(a) {
				this.A.CMa(a);
				if (a.Fe()) {
					if (!this.A.xd) return;
					this.kj(a.Sb());
				}
				for (let b of a.zo()) {
					if (!this.A.xd) break;
					this.bA(b);
				}
			}
			kj(a) {
				this.A.kj(a);
				if (this.A.xd) {
					switch (a.JY()) {
						case 1e3:
							this.BMa(_.TFc(a));
							break;
						case 1001:
							this.CMa(a.OL());
							break;
						case 1022:
							_.fj(a, _.OFc, 1022, _.CY);
							break;
						case 1012:
							a.kR();
							break;
						case 1007:
							_.fj(a, DEc, 1007, _.CY);
							break;
						case 1013:
							this.DMa(_.fj(a, _.NFc, 1013, _.CY));
							break;
						case 1004:
							this.FMa(a.oR());
							break;
						case 1005:
							a.Al();
							break;
						case 1021:
							this.HMa(a.Bl());
							break;
						case 1003:
							this.IMa(a.N7());
							break;
						case 1026:
							this.JMa(_.fj(a, _.JFc, 1026, _.CY));
							break;
						case 1018:
							this.KMa(_.fj(a, _.FFc, 1018, _.CY));
							break;
						case 1023:
							this.OMa(_.fj(a, _.EFc, 1023, _.CY));
							break;
						case 1016:
							this.PMa(_.fj(a, _.DFc, 1016, _.CY));
							break;
						case 1017:
							this.QMa(_.DY(a));
							break;
						case 1015:
							this.RMa(_.fj(a, _.BFc, 1015, _.CY));
							break;
						case 1009:
							this.TMa(a.yja());
							break;
						case 1010:
							this.UMa(_.fj(a, _.yFc, 1010, _.CY));
							break;
						case 1011:
							this.VMa(a.S7());
							break;
						case 1014:
							this.WMa(_.fj(a, _.wY, 1014, _.CY));
							break;
						case 1002:
							this.XMa(a.getText());
							break;
						case 1025:
							this.YMa(_.fj(a, _.vY, 1025, _.CY));
							break;
						case 1006: this.aNa(_.fj(a, _.tFc, 1006, _.CY));
					}
					for (let b of _.mj(a, _.uY, 8, _.oj())) {
						if (!this.A.xd) return;
						this.bA(b);
					}
					for (let b of _.mj(a, FDc, 1020, _.oj())) {
						if (!this.A.xd) break;
						this.EMa(b);
					}
				}
			}
			DMa(a) {
				this.A.DMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			FMa(a) {
				this.A.FMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			HMa(a) {
				this.A.HMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			IMa(a) {
				this.A.IMa(a);
				for (let b of a.zq()) {
					if (!this.A.xd) break;
					this.kj(b);
				}
			}
			JMa(a) {
				this.A.JMa(a);
				var b;
				if ((b = _.zY(a)) == null ? 0 : b.Fe()) {
					if (!this.A.xd) return;
					this.kj(_.zY(a).Sb());
				}
				var c;
				if (((c = _.IFc(a)) == null ? 0 : c.Fe()) && this.A.xd) {
					this.kj(_.IFc(a).Sb());
				}
			}
			KMa(a) {
				this.A.KMa(a);
				for (let b of a.zq()) {
					if (!this.A.xd) break;
					this.kj(b);
				}
			}
			OMa(a) {
				this.A.OMa(a);
				for (let b of a.tB()) {
					if (!this.A.xd) break;
					this.bA(b);
				}
			}
			PMa(a) {
				this.A.PMa(a);
				for (let b of a.zq()) {
					if (!this.A.xd) return;
					this.kj(b);
				}
				for (let b of a.tB()) {
					if (!this.A.xd) break;
					this.bA(b);
				}
			}
			QMa(a) {
				this.A.QMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			RMa(a) {
				this.A.RMa(a);
				if (a.Fe()) {
					if (!this.A.xd) return;
					this.kj(a.Sb());
				}
				var b;
				var c;
				a = (c = (b = _.Z(a, _.xY, 3)) == null ? undefined : b.zo()) != null ? c : [];
				for (let d of a) {
					if (!this.A.xd) break;
					this.bA(d);
				}
			}
			TMa(a) {
				this.A.TMa(a);
				if (a.hasHeader()) {
					if (!this.A.xd) return;
					this.kj(a.getHeader());
				}
				for (let b of _.zFc(a)) {
					if (!this.A.xd) return;
					this.kj(b);
				}
				if (a.aBa() && this.A.xd) {
					this.kj(a.I7());
				}
			}
			UMa(a) {
				this.A.UMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			VMa(a) {
				this.A.VMa(a);
				for (let b of _.wFc(a)) {
					if (!this.A.xd) break;
					this.kj(b);
				}
			}
			WMa(a) {
				this.A.WMa(a);
				for (let b of _.vFc(a)) {
					if (!this.A.xd) break;
					this.kj(b);
				}
			}
			XMa(a) {
				this.A.XMa(a);
				for (let b of a.Xy()) {
					if (!this.A.xd) return;
					this.kj(b);
				}
				for (let b of a.zo()) {
					if (!this.A.xd) break;
					this.bA(b);
				}
			}
			YMa(a) {
				this.A.YMa(a);
				for (let b of a.tB()) {
					if (!this.A.xd) break;
					this.bA(b);
				}
			}
			aNa(a) {
				this.A.aNa(a);
				if (_.sn(a, _.tY, 1)) {
					if (!this.A.xd) return;
					this.kj(a.pR());
				}
				if (_.sn(a, _.tY, 3) && this.A.xd) {
					this.kj(_.Z(a, _.tY, 3));
				}
			}
			bA(a) {
				this.A.bA(a);
				if (this.A.xd) switch (_.jj(a, _.FY)) {
					case 4:
						_.fj(a, _.AEc, 4, _.FY);
						break;
					case 1001:
						_.fj(a, CEc, 1001, _.FY);
						break;
					case 1:
						_.fj(a, _.FEc, 1, _.FY);
						break;
					case 10:
						_.fj(a, _.HEc, 10, _.FY);
						break;
					case 1005:
						_.fj(a, _.KEc, 1005, _.FY);
						break;
					case 1003:
						_.fj(a, _.MEc, 1003, _.FY);
						break;
					case 1002:
						this.GMa(_.fj(a, _.XFc, 1002, _.FY));
						break;
					case 7:
						_.GY(a);
						break;
					case 2:
						this.LMa(_.fj(a, _.WFc, 2, _.FY));
						break;
					case 11:
						_.fj(a, UEc, 11, _.FY);
						break;
					case 8:
						this.MMa(_.fj(a, _.VFc, 8, _.FY));
						break;
					case 1e3:
						this.NMa(_.fj(a, UFc, 1e3, _.FY));
						break;
					case 1004:
						_.fj(a, _.YEc, 1004, _.FY);
						break;
					case 3:
						_.fj(a, dFc, 3, _.FY);
						break;
					case 6:
						_.fj(a, _.oFc, 6, _.FY);
						break;
					case 5:
						_.fj(a, _.pFc, 5, _.FY);
						break;
					case 12:
						this.ZMa(_.fj(a, _.EY, 12, _.FY));
						break;
					case 9: _.fj(a, _.rFc, 9, _.FY);
				}
			}
			GMa(a) {
				this.A.GMa(a);
				for (let b of _.mj(a, _.uY, 2, _.oj())) {
					if (!this.A.xd) return;
					this.bA(b);
				}
				for (let b of _.mj(a, _.uY, 3, _.oj())) {
					if (!this.A.xd) break;
					this.bA(b);
				}
			}
			LMa(a) {
				this.A.LMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			MMa(a) {
				this.A.MMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			NMa(a) {
				this.A.NMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			ZMa(a) {
				this.A.ZMa(a);
				if (a.Fe() && this.A.xd) {
					this.kj(a.Sb());
				}
			}
			EMa(a) {
				this.A.EMa(a);
				if (a.hasAction() && this.A.xd) {
					this.bA(a.getAction());
				}
			}
		};
		var IJc = class {
			constructor() {
				this.A = new _.tY();
			}
			get root() {
				return this.A;
			}
			update(a) {
				if (a.qm()) {
					var b = a.getId();
					if (this.A.getId() === b) return b = this.A, this.A = a, b;
					a = new HJc(a);
					new GJc(a).kj(this.A);
					return a.target;
				}
			}
		};
		var HJc = class {
			constructor(a) {
				this.xd = true;
				this.qk = a;
				this.rk = a.getId();
			}
			bA() {}
			BMa(a) {
				var b = a.QL();
				for (let c = 0; c < b.length; c++) if (b[c].getId() === this.rk) {
					this.target = b[c];
					_.Os(a, 1, _.tY, c, this.qk);
					this.xd = false;
					break;
				}
			}
			CMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.PFc(a, this.qk), this.xd = false;
				}
			}
			kj() {}
			DMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 2, this.qk), this.xd = false;
				}
			}
			EMa() {}
			FMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 1, this.qk), this.xd = false;
				}
			}
			GMa() {}
			HMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 2, this.qk), this.xd = false;
				}
			}
			IMa(a) {
				var b = a.zq();
				for (let c = 0; c < b.length; c++) if (b[c].getId() === this.rk) {
					this.target = b[c];
					_.Os(a, 1, _.tY, c, this.qk);
					this.xd = false;
					break;
				}
			}
			JMa(a) {
				var b = _.IFc(a);
				if (b) {
					let c = b.Sb();
					if ((c == null ? undefined : c.getId()) === this.rk) {
						this.target = c, _.ln(b, _.tY, 1, this.qk), this.xd = false;
					}
				}
				if (a = _.zY(a)) {
					b = a.Sb();
					if ((b == null ? undefined : b.getId()) === this.rk) {
						this.target = b, _.ln(a, _.tY, 2, this.qk), this.xd = false;
					}
				}
			}
			KMa(a) {
				var b = a.zq();
				for (let c = 0; c < b.length; c++) if (b[c].getId() === this.rk) {
					this.target = b[c];
					_.Os(a, 1, _.tY, c, this.qk);
					this.xd = false;
					break;
				}
			}
			LMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 1, this.qk), this.xd = false;
				}
			}
			MMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 1, this.qk), this.xd = false;
				}
			}
			NMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 1, this.qk), this.xd = false;
				}
			}
			OMa() {}
			PMa(a) {
				var b = a.zq();
				for (let c = 0; c < b.length; c++) if (b[c].getId() === this.rk) {
					this.target = b[c];
					_.Os(a, 2, _.tY, c, this.qk);
					this.xd = false;
					break;
				}
			}
			QMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 1, this.qk), this.xd = false;
				}
			}
			RMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 1, this.qk), this.xd = false;
				}
			}
			TMa(a) {
				var b = a.getHeader();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, a.setHeader(this.qk), this.xd = false;
				}
				b = _.zFc(a);
				for (let c = 0; c < b.length; c++) if (b[c].getId() === this.rk) {
					this.target = b[c];
					_.Os(a, 1, _.tY, c, this.qk);
					this.xd = false;
					break;
				}
				b = a.I7();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 4, this.qk), this.xd = false;
				}
			}
			UMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 2, this.qk), this.xd = false;
				}
			}
			VMa(a) {
				var b = _.wFc(a);
				for (let c = 0; c < b.length; c++) if (b[c].getId() === this.rk) {
					this.target = b[c];
					_.Os(a, 1, _.tY, c, this.qk);
					this.xd = false;
					break;
				}
			}
			WMa(a) {
				var b = _.vFc(a);
				for (let c = 0; c < b.length; c++) if (b[c].getId() === this.rk) {
					this.target = b[c];
					_.Os(a, 1, _.tY, c, this.qk);
					this.xd = false;
					break;
				}
			}
			XMa(a) {
				var b = a.Xy();
				for (let c = 0; c < b.length; c++) if (b[c].getId() === this.rk) {
					this.target = b[c];
					_.Os(a, 4, _.tY, c, this.qk);
					this.xd = false;
					break;
				}
			}
			YMa() {}
			ZMa(a) {
				var b = a.Sb();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 1, this.qk), this.xd = false;
				}
			}
			aNa(a) {
				var b = a.pR();
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, a.setHost(this.qk), this.xd = false;
				}
				b = _.Z(a, _.tY, 3);
				if ((b == null ? undefined : b.getId()) === this.rk) {
					this.target = b, _.ln(a, _.tY, 3, this.qk), this.xd = false;
				}
			}
		};
		_.wZ = function(a, b) {
			if (!b.qm()) return _.Ef;
			var c = a.A.get(b);
			if (!c) {
				c = { oLb: new _.ef((d) => {
					if (c) {
						c.next = (e) => {
							d.next(e);
						};
					}
					return () => {
						d.complete();
						a.A.delete(b);
					};
				}).pipe(_.Xg()) }, a.A.set(b, c);
			}
			return c.oLb;
		};
		_.xZ = class {
			constructor() {
				this.tree = new IJc();
				this.F = new _.ml();
				this.A = new WeakMap();
			}
			reset() {
				this.tree = new IJc();
				this.F.next();
				this.A = new WeakMap();
			}
			get root() {
				return this.tree.root;
			}
			update(a) {
				var b = this.tree.root;
				var c = this.tree.update(a);
				if (!c) return false;
				if (c === b) {
					this.F.next();
				}
				var d;
				var e;
				if (!((d = this.A.get(c)) == null || (e = d.next) == null)) {
					e.call(d, a);
				}
				return true;
			}
			has(a) {
				return this.A.has(a);
			}
		};
		_.xZ.J = function(a) {
			return new (a || _.xZ)();
		};
		_.xZ.sa = _.Cd({
			token: _.xZ,
			factory: _.xZ.J
		});
		var JJc;
		var KJc;
		var LJc;
		var MJc;
		var NJc;
		var yZ;
		var OJc;
		var QJc;
		JJc = new WeakSet();
		KJc = function(a, b) {
			var c;
			var d;
			var e = _.MX((d = (c = a.child()) == null ? undefined : c.id()) != null ? d : a.xb().getId());
			if (e) {
				var f;
				var g = (f = a.R()) == null ? undefined : f.nativeElement;
				if (g) {
					g.setAttribute("id", e), b(() => {
						g.removeAttribute("id");
					});
				}
			}
		};
		LJc = function(a) {
			var b = a.R();
			var c;
			if (!b || !_.SFc(a.xb()) || ((c = a.child()) == null ? 0 : c.QMb)) {
				let g;
				if (!((g = a.F) == null)) {
					g.Ba();
				}
				a.F = undefined;
				a.ea = undefined;
			} else {
				if (a.ea !== b) {
					a.F && a.F.Ba(), a.ea = b, b = _.Xi({
						parent: a.Pa,
						vd: [{
							Da: _.Jf,
							Vc: b
						}, _.HC]
					}), a.F = _.ke(b, () => _.Qd(() => {
						var g = new jZ();
						g.Rb();
						return g;
					}));
				}
				a.F.xb.set(a.xb());
				var d;
				var e;
				var f;
				a.F.disabled.set((f = (d = a.child()) == null ? undefined : (e = d.disabled) == null ? undefined : e.call(d)) != null ? f : false);
			}
		};
		MJc = function(a) {
			var b = a.aa();
			return b && b.location.nativeElement.isConnected && (b = b.Pa.get(_.$h, null)) ? b : (b = a.Pab()) && b.element.nativeElement.isConnected ? b : a.bf;
		};
		NJc = function(a, b) {
			for (let c of b) a.U.Zg(c, MJc(a));
		};
		yZ = function(a, b, c, d) {
			if (c !== null) {
				var e;
				var f = (e = a.R()) == null ? undefined : e.nativeElement;
				if (f && f.getAttribute(b) !== c) {
					f instanceof HTMLElement ? BCc(f, b, c) : DCc(f, b, c), d(() => {
						if (f.getAttribute(b) === c) {
							f.removeAttribute(b);
						}
					});
				}
			}
		};
		OJc = function(a, b, c, d) {
			if (c !== null) {
				var e;
				var f = (e = a.R()) == null ? undefined : e.nativeElement;
				if (f) {
					var g = (f.getAttribute(b) || "").split(/\s+/).filter((p) => p);
					var k = c.split(/\s+/).filter((p) => p).filter((p) => !g.includes(p));
					if (k.length !== 0) {
						a = [...g, ...k].join(" "), f instanceof HTMLElement ? BCc(f, b, a) : DCc(f, b, a), d(() => {
							var p = (f.getAttribute(b) || "").split(/\s+/).filter((r) => r).filter((r) => !k.includes(r));
							if (p.length > 0) {
								p = p.join(" "), f instanceof HTMLElement ? BCc(f, b, p) : DCc(f, b, p);
							} else {
								f.removeAttribute(b);
							}
						});
					}
				}
			}
		};
		PJc = function(a, b, c, d) {
			b = new Map(b);
			d = `${d}[`;
			for (let e of c.keys()) if (e.startsWith(d) && e.endsWith("]")) {
				let f = e.substring(d.length, e.length - 1);
				let g = uZ(a.A, e, null, c);
				if (g === null) {
					b.delete(f);
				} else {
					b.set(f, g);
				}
			}
			return b;
		};
		QJc = function(a, b) {
			var c;
			var d;
			var e;
			return (e = (c = a.child()) == null ? undefined : (d = c.a7b) == null ? undefined : d.includes(b)) != null ? e : false;
		};
		_.zZ = class {
			constructor() {
				this.xb = _.Li.required();
				this.Z0a = _.Ki();
				this.aa = _.M();
				this.hidden = _.W(() => {
					var c;
					var d = (c = this.A.evaluate("hidden", "bool", this.xb().lja(), _.AY(this.xb()))) != null ? c : false;
					this.Z0a.emit(d);
					return d;
				});
				this.ma = _.W(() => this.A.evaluate("pan-latency", "string", "", _.AY(this.xb())));
				this.Oab = _.Oi();
				this.child = _.W(() => {
					var c;
					var d;
					return (d = (c = this.aa()) == null ? undefined : c.instance) != null ? d : this.Oab();
				});
				this.H = _.W(() => {
					var c;
					var d;
					var e = (d = (c = this.child()) == null ? undefined : c.Lg()) != null ? d : this.Ga;
					c = e == null ? undefined : e.nativeElement;
					if (c instanceof HTMLElement || c instanceof SVGElement) return e;
				});
				this.R = _.W(() => {
					var c;
					var d;
					var e;
					var f = (e = (c = this.child()) == null ? undefined : (d = c.oj) == null ? undefined : d.call(c)) != null ? e : this.H();
					c = f == null ? undefined : f.nativeElement;
					if (c instanceof HTMLElement || c instanceof SVGElement) return f;
				});
				this.Pa = _.m(_.Xf);
				this.U = _.m(_.kZ);
				this.A = _.m(_.vZ);
				this.X = _.m(_.jP);
				this.veLoggingService = _.m(_.Jk);
				this.yb = _.m(_.th);
				this.Dc = _.m(_.cm);
				this.fa = _.m(_.Vk);
				this.na = _.m(_.Wk);
				this.Ga = _.m(_.Jf);
				this.bf = _.m(_.$h);
				this.Pab = _.Oi(Object.assign({}, {}, { read: _.$h }));
				var a = _.m(_.xZ);
				var b = _.m(_.Hu);
				_.Kg((c) => {
					if (this.xb().qm()) {
						var d = this.xb();
						_.Qd(() => {
							var e = _.wZ(a, d).subscribe(() => {
								b.lb();
							});
							c(() => {
								e.unsubscribe();
							});
						});
					}
				});
				_.cj({ write: (c) => {
					var d;
					var e = (d = this.H()) == null ? undefined : d.nativeElement;
					if (e) {
						(d = this.ma()) ? e.setAttribute("pan-latency", d) : e.removeAttribute("pan-latency");
					}
					var f;
					if (e = (f = this.H()) == null ? undefined : f.nativeElement) {
						e.removeAttribute("style");
						for (var g of [...e.classList]) g.startsWith("sdui--") && e.classList.remove(g);
						f = this.xb();
						g = PJc(this, _.Yo(f, 2), _.AY(f), "styles");
						for (let [v, w] of g) {
							g = v;
							d = w;
							if (QJc(this, g)) continue;
							if (d = _.iP(this.X, d)) RJc.has(g) ? (e.style.setProperty(`--sdui--${g}`, d), e.classList.add(`sdui--${g}`)) : e.style.setProperty(g, d);
						}
						g = PJc(this, _.Yo(f, 3), _.AY(f), "hover_styles");
						for (let [v, w] of g) {
							g = v;
							d = w;
							if (QJc(this, g)) continue;
							if ((d = _.iP(this.X, d)) && RJc.has(g)) {
								e.style.setProperty(`--sdui--${g}--hover`, d), e.classList.add(`sdui--${g}--hover`);
							}
						}
						f = PJc(this, _.Yo(f, 4), _.AY(f), "focus_styles");
						for (let [v, w] of f) {
							f = v;
							g = w;
							if (QJc(this, f)) continue;
							if ((g = _.iP(this.X, g)) && RJc.has(f)) {
								e.style.setProperty(`--sdui--${f}--focus`, g), e.classList.add(`sdui--${f}--focus`);
							}
						}
					}
					e = this.xb();
					if ((f = _.Z(e, sFc, 7)) && (e = this.H()) && f.qm() && this.oa !== f) {
						this.oa = f;
						f = {
							ve: f.getId(),
							veMetadata: f.getMetadata(),
							cra: _.Pm(f, 3),
							yda: _.Pm(f, 4),
							Zqa: _.Pm(f, 5),
							ara: _.Pm(f, 6),
							bra: _.Pm(f, 7),
							T1: _.Pm(f, 8),
							dra: _.Pm(f, 9)
						};
						var k;
						this.I = (k = this.I) != null ? k : new _.Bz(e, this.veLoggingService, this.yb, this.Dc, this.fa, this.na);
						Object.assign(this.I, f);
						this.I.Wb();
						this.I.ib();
					}
					var p;
					if (k = (p = this.R()) == null ? undefined : p.nativeElement) {
						p = _.BY(this.xb());
						var r;
						if (this.A.evaluate("aria.hidden", "bool", (r = p == null ? undefined : p.lja()) != null ? r : false, _.AY(this.xb()))) {
							yZ(this, "aria-hidden", "true", c);
						}
						r = uZ(this.A, "aria.expanded", (p == null ? 0 : _.vn(p, 8)) ? _.Pm(p, 8) : null, _.AY(this.xb()));
						if (r != null) {
							yZ(this, "aria-expanded", r.toString(), c);
						}
						if (uZ(this.A, "aria.required", (p == null ? 0 : _.vn(p, 6)) ? _.Pm(p, 6) : null, _.AY(this.xb()))) {
							yZ(this, "aria-required", "true", c), yZ(this, "required", "", c);
						}
						r = uZ(this.A, "aria.selected", (p == null ? 0 : _.vn(p, 1001)) ? p.Mw() : null, _.AY(this.xb()));
						if (r != null) {
							yZ(this, "aria-selected", r.toString(), c);
						}
						if (p) {
							OJc(this, "aria-controls", _.NX(_.l(p, 7) || null), c), OJc(this, "aria-describedby", _.NX(_.l(p, 5) || null), c), OJc(this, "aria-labelledby", _.NX(_.l(p, 4) || null), c), yZ(this, "aria-label", p.Bl() || null, c), k.getAttribute("role") !== "menuitem" && yZ(this, "role", p.Bq() || null, c), yZ(this, "aria-valuemax", _.Um(p, 9) ? _.Vm(p, 9).toString() : null, c), yZ(this, "aria-valuemin", _.Um(p, 10) ? _.Vm(p, 10).toString() : null, c), yZ(this, "aria-valuenow", _.Um(p, 11) ? _.Vm(p, 11).toString() : null, c);
						} else {
							if (k.getAttribute("role") !== "menuitem") {
								yZ(this, "role", null, c);
							}
						}
					}
					KJc(this, c);
				} });
				_.cj({ write: () => {
					LJc(this);
				} });
				_.cj({ rma: (c) => {
					if (!this.hidden()) {
						var d = this.xb();
						if (d.JY() === 0 || this.H()) {
							var e = _.mj(d, _.uY, 8, _.oj());
							if (e.length !== 0) {
								var f = false;
								c(() => {
									f = true;
								});
								queueMicrotask(() => {
									if (!f) for (let g of e) JJc.has(g) || (this.U.Zg(g, MJc(this)), JJc.add(g));
								});
							}
						}
					}
				} });
				_.cj({ write: (c) => {
					var d = this.xb();
					var { KKb: e, LKb: f, DDb: g, Lxb: k } = GDc(d);
					if (e.length !== 0 || f.length !== 0 || g.length !== 0 || k.length !== 0) {
						var p;
						if (d = (p = this.H()) == null ? undefined : p.nativeElement) {
							var r;
							if (e.length > 0) {
								let v = _.Af(d, "mouseenter").subscribe(() => {
									clearTimeout(r);
									r = setTimeout(() => {
										NJc(this, e);
										r = undefined;
									}, 200);
								});
								c(() => {
									v.unsubscribe();
								});
							}
							if (f.length > 0 || e.length > 0) {
								let v = _.Af(d, "mouseleave").subscribe(() => {
									clearTimeout(r);
									r = undefined;
									for (let w of f) try {
										this.U.Zg(w, MJc(this));
									} catch (D) {
										if (!_.Dr(w, _.AEc, 4, _.FY)) throw D;
									}
								});
								c(() => {
									v.unsubscribe();
								});
							}
							if (g.length > 0) {
								let v = _.Af(d, "focus").subscribe(() => {
									NJc(this, g);
								});
								c(() => {
									v.unsubscribe();
								});
							}
							if (k.length > 0) {
								let v = _.Af(d, "blur").subscribe(() => {
									NJc(this, k);
								});
								c(() => {
									v.unsubscribe();
								});
							}
							c(() => {
								clearTimeout(r);
								r = undefined;
							});
						}
					}
				} });
			}
			Ba() {
				if (this.I) {
					this.I.Ba();
				}
				var a;
				if (!((a = this.F) == null)) {
					a.Ba();
				}
			}
		};
		_.zZ.J = function(a) {
			return new (a || _.zZ)();
		};
		_.zZ.Oa = _.We({
			type: _.zZ,
			da: [[
				"",
				"sdui-component",
				""
			]],
			Ud: function(a, b, c) {
				if (a & 1) {
					_.ii(c, b.Oab, _.kP, 5)(c, b.Pab, _.kP, 5, _.$h);
				}
				if (a & 2) {
					_.ki(2);
				}
			},
			Ua: 2,
			Ja: function(a, b) {
				if (a & 2) {
					_.P("sdui-hidden", b.hidden());
				}
			},
			inputs: { xb: [1, "comp"] },
			outputs: { Z0a: "hiddenOutput" }
		});
		var RJc = new Set("background-color border-bottom-color border-bottom-style border-bottom-width border-left-color border-left-style border-left-width border-right-color border-right-style border-right-width border-top-color border-top-style border-top-width box-shadow color cursor outline-color outline-style outline-width text-shadow".split(" "));
		SJc.d3 = d3 || {};
		_.TJc = class {};
		_.AZ = class {
			constructor() {
				var a = _.m(_.zZ);
				var b = _.m(_.TJc);
				var c = _.m(_.ag);
				var d = _.m(_.Hg);
				var e = _.m(_.Xf);
				var f = _.m(_.$h);
				var g = new UJc(a);
				c.Hc(() => {
					g.value = undefined;
				});
				_.Kg((k) => {
					var p = a.xb();
					if (p) if (a.hidden()) g.value = undefined;
					else {
						var r = false;
						k(() => {
							r = true;
						});
						b.A(p).then((v) => {
							if (!r) if (v) {
								var w = g.value;
								var D = HDc(f, w, v.primitiveType, e, d);
								for (let [L, N] of v.inputs.entries()) D.zk(L, N);
								var G = D.A;
								if (D === w && f.indexOf(G) === -1) {
									f.insert(G);
								}
								k(() => {
									var L = f.indexOf(G);
									if (L >= 0) {
										f.detach(L);
									}
								});
								g.value = D;
							} else g.value = undefined;
						});
					}
				});
			}
		};
		_.AZ.J = function(a) {
			return new (a || _.AZ)();
		};
		_.AZ.Oa = _.We({
			type: _.AZ,
			da: [[
				"",
				"dynamic-sdui-component",
				""
			]],
			features: [_.mh([{
				directive: _.zZ,
				inputs: ["comp", "comp"],
				outputs: ["hiddenOutput", "hiddenOutput"]
			}])]
		});
		var UJc = class {
			constructor(a) {
				this.A = a;
			}
			get value() {
				return this.bL;
			}
			set value(a) {
				if (this.bL !== a) {
					let b;
					if (!((b = this.bL) == null)) {
						b.destroy();
					}
					this.bL = a;
				}
				this.A.aa.set(a);
			}
		};
		var VJc = class {};
		var WJc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getAction() {
				return _.Z(this, _.uY, 2);
			}
			hasAction() {
				return _.sn(this, _.uY, 2);
			}
		};
		var XJc = _.Yc([
			0,
			_.zt,
			KY
		]);
		var YJc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getError() {
				return _.l(this, 1);
			}
			hasError() {
				return _.zn(this, 1);
			}
		};
		var ZJc = _.Xc(YJc, [0, _.zt]);
		var $Jc = class extends _.Aj {
			constructor(a, b) {
				super();
				this.socket = null;
				this.A = typeof a !== "string" ? a : XIc(b, a);
				_.ck(this, () => {
					var c;
					if (!((c = this.socket) == null)) {
						c.close();
					}
					this.socket = null;
				});
			}
			getSocket() {
				if (!this.socket) {
					let a = bIc(this.A);
					this.socket = a;
					THc(a, () => {
						this.socket = null;
					});
				}
				return this.socket;
			}
		};
		var aKc;
		var cKc = function(a) {
			var b;
			var c = _.cw(() => {
				a.reportError(new rIc());
				a.close();
			}, (b = a.options.yxa) != null ? b : 3e3);
			bKc(a, () => {
				_.ha.clearTimeout(c);
			});
			a.on("ERROR", () => {
				_.ha.clearTimeout(c);
			});
		};
		var bKc = function(a, b) {
			if (a.socket.getState() === 5) {
				b();
			} else {
				a.socket.getState() === 3 ? a.eventHandler.ZZ(a.socket, yHc, b) : (a.reportError(new fZ("Connection has closed.")), a.getState() !== "CLOSED" && a.close());
			}
		};
		var dKc = class extends HIc {
			constructor(a, b, c, d = {}) {
				aKc = aKc === undefined ? 1 : aKc + 2;
				super(a, aKc, b);
				this.options = d;
				this.eventHandler = new _.ok(this);
				this.JX = c;
				_.bk(this, this.eventHandler);
			}
			sendHeaders(a) {
				cKc(this);
				LDc(() => {
					this.Yf("OPEN");
					bKc(this, () => {
						super.sendHeaders(a);
					});
				});
			}
			g8a(a) {
				if (_.sn(a, _.lw, 3)) {
					FIc(this, "STATUS", a.Pc());
				} else {
					this.reportError(new fZ("Did not receive status from server endpoint."));
				}
			}
			emit(a, b) {
				LDc(() => {
					bKc(this, () => {
						super.emit(a, b);
					});
				});
			}
			close() {
				LDc(() => {
					if (this.socket.getState() === 1) {
						this.Yf("CLOSED");
					} else {
						super.close();
					}
				});
			}
		};
		var gKc = function(a, b, c) {
			_.x(function* () {
				var d = {
					nJa: _.l(a, 1),
					id: _.l(a, 4)
				};
				c.e$.resolve(d);
				c.sendHeaders(a);
				c.emit(b, true);
			});
		};
		var hKc = class extends _.Rj {
			constructor(a, b, c = {}) {
				super();
				this.getSocket = a;
				this.Wn = b;
				this.options = c;
				JDc(this.options.yxa);
			}
		};
		var iKc = function(a, b) {
			for (let c of b) {
				let d = a;
				a = (e) => c.intercept(e, d);
			}
			return a;
		};
		var jKc = function(a, b, c, d, e) {
			var f = _.Yn();
			var g = {
				nJa: b,
				id: f
			};
			b = jIc(iIc(new bZ(), b), f);
			LX();
			KDc({
				metricType: 1001,
				metadata: g
			});
			var k = new _.gw();
			a = fKc(a);
			a.on("DATA", (p) => {
				k.resolve(p);
			});
			a.on("ERROR", (p) => {
				k.reject(p);
			});
			a.on("STATUS", (p) => {
				if (p.Ff() !== 0) {
					k.reject(new tIc(p)), LX(), LX();
				}
				LX();
				KDc({
					metricType: 1002,
					metadata: g
				});
				KDc({
					metricType: 1004,
					metadata: e
				});
			});
			gKc(b, c, a);
			return Promise.race([k.promise, NDc(d)]);
		};
		var kKc = class extends hKc {
			call(a, b, c = {}) {
				JDc(c.requestTimeout);
				var d = {
					nJa: a,
					id: _.Yn()
				};
				KDc({
					metricType: 1003,
					metadata: d
				});
				var e;
				var f = iKc((g) => jKc(this, a, g, c, d), (e = this.options.Hqa) != null ? e : [])(b);
				f.catch(() => {
					LX();
				});
				return f;
			}
		};
		var lKc = class extends _.Aj {
			constructor(a, b, c) {
				super();
				this.client = typeof a === "string" ? new $Jc(a, b) : new $Jc(a);
				a = typeof a === "string" ? c : b;
				this.HCb = new kKc(() => this.client.getSocket(), new LHc().create(YJc, XJc, ZJc, {
					Moa: c == null ? undefined : c.Moa,
					Wn: c == null ? undefined : c.Wn
				}), a);
			}
			Zg(a, b = {}) {
				return this.HCb.call("ActionExecutorService.executeAction", a, b);
			}
			dispose() {
				this.client.dispose();
			}
		};
		lKc.serviceId = new class extends VJc {
			getId() {
				return "ActionExecutorService";
			}
		}();
		var mKc = class {};
		var nKc;
		var oKc;
		nKc = function(a, b) {
			return _.x(function* () {
				var c;
				var d = (c = a.config) == null ? undefined : c.H;
				if (!d || !a.client) throw Error("uh");
				c = yield a.client;
				var e = c.Zg;
				var f = new WJc();
				d = _.Lj(f, 1, d);
				d = _.ln(d, _.uY, 2, b);
				c = yield e.call(c, d);
				if (c.hasError()) throw Error(c.getError());
			});
		};
		oKc = function(a, b) {
			_.x(function* () {
				var c = new _.uY();
				var d = new SEc();
				d = _.Lj(d, 1, b.url);
				c = _.Ap(c, 7, _.FY, d);
				return nKc(a, c);
			});
		};
		_.BZ = class {
			constructor() {
				this.config = _.m(mKc, { optional: true });
				if (this.config != null) if (_.cP(QGc)) this.client = _.m(iZ).kL(lKc);
				else {
					let a = _.m(hZ).kL(lKc, this.config.F());
					this.client = Promise.resolve(a);
					_.m(_.ag).Hc(() => {
						a.dispose();
					});
				}
			}
		};
		_.BZ.J = function(a) {
			return new (a || _.BZ)();
		};
		_.BZ.sa = _.Cd({
			token: _.BZ,
			factory: _.BZ.J
		});
		var CZ = class {};
		CZ.J = function(a) {
			return new (a || CZ)();
		};
		CZ.sa = _.Cd({
			token: CZ,
			factory: CZ.J,
			wa: "root"
		});
		_.pKc = function(a) {
			if (!a) return null;
			try {
				var b = _.uw(a);
			} catch (c) {
				return null;
			}
			return b.F ? {
				type: "EXTERNAL",
				url: a
			} : {
				type: "INTERNAL",
				url: a
			};
		};
		_.DZ = class {
			constructor() {
				this.I = _.m(OY);
				this.H = _.m(_.fP);
				this.A = _.m(_.Cl);
				this.F = _.m(_.BZ);
				_.m(CZ);
			}
			navigate(a, b = { Sq: false }) {
				a = b.Sq ? Object.assign({}, a, { type: "EXTERNAL" }) : a;
				if (b.Sq) b = "SELF";
				else {
					var c;
					var d;
					b = (d = (c = this.F.config) == null ? undefined : c.A(a)) != null ? d : "SELF";
				}
				c = b;
				switch (c) {
					case "DELEGATE":
						oKc(this.F, a);
						break;
					case "SELF":
						switch (a.type) {
							case "INTERNAL":
								var e = _.pA(this.A, a.url);
								var f, g;
								a = (g = (f = e.root.children.primary) == null ? undefined : f.segments) != null ? g : [];
								f = ["/"];
								for (var k of a) f.push(k.path), _.kqa(k.parameters) || f.push(k.parameters);
								e = this.A.bk(f, {
									Vq: "merge",
									queryParams: e.queryParams
								});
								f = _.qA(this.A, e);
								k = new URL(this.A.url, location.origin);
								k.searchParams.delete("sduiData");
								f = new URL(f, location.origin);
								f.searchParams.delete("sduiData");
								k.pathname !== f.pathname ? k = false : (k.searchParams.sort(), f.searchParams.sort(), k = k.searchParams.toString() === f.searchParams.toString());
								_.br(this.A, e, {
									replaceUrl: k,
									state: { ignoreNavigation: k }
								});
								break;
							case "EXTERNAL":
								g = k = _.uw(a.url);
								if (PDc(g)) {
									f = _.uw(this.H.ref.location.toString());
									k = g.clone();
									f = f.A;
									d = g.A;
									for (e of NGc) _.jZa(f, e) && !_.jZa(d, e) && (g = k, a = e, c = f.getValues(e), Array.isArray(c) || (c = [String(c)]), _.kZa(g.A, a, c));
									e = k;
								} else e = g;
								k = MGc(this.I, e);
								_.rd(window, k.toString(), "_blank");
								break;
							default: _.sb(a.type, undefined);
						}
						break;
					default: _.sb(c, undefined);
				}
			}
		};
		_.DZ.J = function(a) {
			return new (a || _.DZ)();
		};
		_.DZ.sa = _.Cd({
			token: _.DZ,
			factory: _.DZ.J
		});
		_.EZ = class {};
		var qKc = function(a) {
			if (!a) return "";
			var b = "";
			for (let c of a.children || []) {
				let d = c.routeConfig;
				if (d && d.path && d.Af !== "primary" && d.Af) {
					b += `(${d.Af}:${d.path})`;
				} else {
					d && d.path ? b += "/" + d.path : d && d.PJb && (b += "/" + d.PJb);
				}
			}
			return b + qKc(a.firstChild);
		};
		var rKc = function() {
			var a = _.Jk.prototype;
			var b = sharedHostData.veLoggingService;
			if (a) {
				var c = Object.getOwnPropertyNames(a);
				for (let d of c) d in b && Reflect.defineProperty(a, d, {
					get() {
						var e = b[d];
						return e instanceof Function ? e.bind(b) : e;
					},
					set(e) {
						b[d] = e;
					}
				});
				a = undefined;
			}
		};
		var sKc = class extends _.h {
			constructor(a) {
				super(a);
			}
			getWidth() {
				return _.ct(this, 1);
			}
		};
		var tKc = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var uKc = _.bd(_.sJ);
		_.FZ = class {
			constructor(a) {
				this.A = a.A ? (0, _.d4b)(a.A) : new _.MP();
				this.Fl = !!_.ek(this.A, 2, _.gb);
			}
		};
		_.FZ.A = false;
		_.FZ.J = function(a) {
			return new (a || _.FZ)(_.ae(_.JP));
		};
		_.FZ.sa = _.Cd({
			token: _.FZ,
			factory: _.FZ.J,
			wa: "root"
		});
		var GZ = class {
			constructor(a) {
				this.A = "";
				this.A = qKc(a.Pp.root.snapshot) || "pan-unresolved-page-path";
				Promise.resolve(this.A);
				this.F = this.A;
				a.events.subscribe((b) => {
					var c = false;
					if (b instanceof _.Meb) {
						new Promise((d) => {
							this.H = d;
						});
					} else {
						b instanceof _.Neb ? (this.F = this.A, this.A = qKc(b.state.root)) : b instanceof _.Al || b instanceof _.zl ? (this.A = this.F, c = true) : b instanceof _.yl && (c = true);
					}
					if (c && this.H) {
						this.H(this.A);
					}
				});
			}
			getPath() {
				return this.A;
			}
		};
		GZ.J = function(a) {
			return new (a || GZ)(_.ae(_.Cl));
		};
		GZ.sa = _.Cd({
			token: GZ,
			factory: GZ.J,
			wa: "root"
		});
		var HZ = class {
			constructor(a, b, c, d, e) {
				this.H = b;
				this.F = c;
				this.A = e;
				this.isInternal = false;
				this.organizationId = this.folderId = this.projectNumber = null;
				this.authUser = (a = a.Fl ? _.Z(a.A, _.Z3b, 1, _.gb) : null) ? _.ek(a, 1, _.gb) : null;
				a = a ? _.ek(a, 4, _.gb) : null;
				if (_.FZ.A || a && (a.startsWith("pantheon.prober") || a.startsWith("pantheon.vmprober"))) this.isInternal = true;
				this.Noa = d.Noa;
				this.rna = d.rna;
				this.u2a = _.e1b(_.a4b);
				this.language = (a = (this.H.ref.pantheon_locale || "").match(/^\w{2,3}([-_]|$)/)) ? a[0].replace(/[_-]/g, "") : "";
				var f;
				this.Mla = (f = d.Mla) != null ? f : "global";
				var g;
				this.Fga = (g = d.Fga) != null ? g : "";
			}
			L7() {
				return {
					path: this.getPath(),
					language: this.language,
					Noa: this.Noa,
					rna: this.rna,
					isInternal: this.isInternal,
					sessionId: this.A.H.get(),
					J4: this.A.IY(),
					f6b: this.A.F.get(),
					projectNumber: this.projectNumber,
					folderId: this.folderId,
					organizationId: this.organizationId,
					u2a: this.u2a,
					Mla: this.Mla,
					Fga: this.Fga
				};
			}
			getPath() {
				var a = _.rP().currentPageView.path;
				return a !== "pan-unresolved-page-path" ? a : this.F.getPath();
			}
		};
		HZ.J = function(a) {
			return new (a || HZ)(_.ae(_.FZ), _.ae(_.fP), _.ae(GZ), _.ae(_.JP), _.ae(_.RP));
		};
		HZ.sa = _.Cd({
			token: HZ,
			factory: HZ.J,
			wa: "root"
		});
		var vKc = class extends _.jz {
			constructor(a) {
				super(a);
			}
		};
		var wKc = class {
			constructor(a, b) {
				this.A = a;
				this.F = b;
			}
			Hha(a) {
				return new vKc(a);
			}
			ega() {}
			DA(a) {
				a: {
					var b = _.l(a, 8);
					if (b) {
						var c = uKc(b);
						break a;
					}
					c = new _.sJ();
				}
				c = _.cn(c, 23, 30);
				c = _.Lj(c, 12, "CLOUD_CONSOLE");
				c = _.cn(c, 116, 31);
				b = this.F.getPath();
				c = _.Lj(c, 13, b);
				b = new sKc();
				b = _.Ym(b, 1, this.A.ref.screen.width);
				b = _.Ym(b, 2, this.A.ref.screen.height);
				var d = new sKc();
				d = _.Ym(d, 1, this.A.ref.innerWidth);
				d = _.Ym(d, 2, this.A.ref.innerHeight);
				var e = new tKc();
				b = _.ln(e, sKc, 1, b);
				b = _.ln(b, sKc, 3, d);
				b = _.Ym(b, 2, this.A.ref.screen.colorDepth);
				c = _.ln(c, tKc, 33, b);
				c = _.Mj(c, 16, this.A.ref.location.protocol === "https:");
				c = _.Lj(c, 17, this.A.ref.location.hostname);
				c = _.Mj(c, 27, this.A.ref.document.hidden);
				b = this.F.L7().J4;
				c = _.Lj(c, 34, b);
				b = this.F.L7().isInternal;
				c = _.Mj(c, 6, b);
				b = c.DA;
				d = new _.rJ();
				d = _.pt(d, 3, _.d2b, 724);
				c = b.call(c, d.setValue("true"));
				_.fWa(a, c.serialize());
			}
		};
		var IZ = class extends _.Jy {
			constructor() {
				super();
				this.A = null;
				this.H = _.m(_.fP);
				this.I = _.m(HZ);
			}
			F() {
				if (this.A) return this.A;
				var a = new _.Nzb();
				var b = new wKc(this.H, this.I);
				var c = new _.ak(509, "0");
				c.F = 2500;
				c = c.build();
				this.A = new _.yJ(a, b, c);
				_.szb(this.A);
				this.A.Hr("hover", 9);
				this.A.Hr("scroll", 22);
				this.A.Hr("drag", 30);
				this.A.Hr("input", 15);
				return this.A;
			}
		};
		IZ.logger = null;
		IZ.J = function(a) {
			return new (a || IZ)();
		};
		IZ.sa = _.Cd({
			token: IZ,
			factory: IZ.J,
			wa: "root"
		});
		var JZ = class extends _.Jk {
			constructor() {
				super(_.m(IZ), JZ.Ydb, _.m(_.th, { optional: true }));
				this.eMb = _.m(_.KP);
				_.m(IZ);
				this.yb = _.m(_.th, { optional: true });
				this.bwa = new _.Jk(this.eMb, JZ.Ydb, this.yb);
				var a = this;
				return new Proxy(this, { get(b, c) {
					var d = Reflect.get(b, c, b);
					if (typeof d === "function") {
						let e = Reflect.get(b.bwa, c, b.bwa);
						return new Proxy(d, { apply: (f, g, k) => {
							Reflect.apply(e, a.bwa, k);
							return Reflect.apply(f, g, k);
						} });
					}
					return d;
				} });
			}
		};
		JZ.Ydb = new _.Bcb();
		JZ.J = function(a) {
			return new (a || JZ)();
		};
		JZ.sa = _.Cd({
			token: JZ,
			factory: JZ.J,
			wa: "root"
		});
		var xKc = new _.hP("45732747");
		var KZ = class {};
		KZ.J = function(a) {
			return new (a || KZ)();
		};
		KZ.qc = _.Ve({ type: KZ });
		KZ.oc = _.Dd({
			vd: [{
				Da: _.Wk,
				Vc: {
					ddb: !_.aP(),
					oBb: _.aP()
				}
			}, {
				Da: _.Jk,
				Mf: location.pathname.startsWith("/macro-lounge") || _.cP(xKc) ? JZ : _.Jk
			}],
			imports: [{
				bN: _.Cz,
				vd: _.Psa()
			}, _.Cz]
		});
		if (_.aP()) {
			rKc();
		}
		_.LZ = class {
			constructor() {
				this.xb = _.Li.required();
				this.hidden = _.M(false);
			}
		};
		_.LZ.J = function(a) {
			return new (a || _.LZ)();
		};
		_.LZ.ka = _.u({
			type: _.LZ,
			da: [["sdui-component"]],
			Ua: 4,
			Ja: function(a, b) {
				if (a & 2) {
					_.pi("display", "contents"), _.P("sdui-hidden", b.hidden());
				}
			},
			inputs: { xb: [1, "comp"] },
			ha: 1,
			ia: 1,
			la: [[
				"dynamic-sdui-component",
				"",
				3,
				"hiddenOutput",
				"comp"
			]],
			template: function(a, b) {
				if (a & 1) {
					_.Gh(0, 0), _.J("hiddenOutput", function(c) {
						b.hidden.set(c);
					}), _.Hh();
				}
				if (a & 2) {
					_.E("comp", b.xb());
				}
			},
			dependencies: [
				_.tz,
				_.AZ,
				KZ
			],
			styles: ["[_nghost-%COMP%]     .sdui--background-color{background-color:var(--sdui--background-color)!important}[_nghost-%COMP%]     .sdui--background-color--hover:hover{background-color:var(--sdui--background-color--hover)!important}[_nghost-%COMP%]     .sdui--background-color--focus:focus-visible{background-color:var(--sdui--background-color--focus)!important}[_nghost-%COMP%]     .sdui--border-bottom-color{border-bottom-color:var(--sdui--border-bottom-color)!important}[_nghost-%COMP%]     .sdui--border-bottom-color--hover:hover{border-bottom-color:var(--sdui--border-bottom-color--hover)!important}[_nghost-%COMP%]     .sdui--border-bottom-color--focus:focus-visible{border-bottom-color:var(--sdui--border-bottom-color--focus)!important}[_nghost-%COMP%]     .sdui--border-bottom-style{border-bottom-style:var(--sdui--border-bottom-style)!important}[_nghost-%COMP%]     .sdui--border-bottom-style--hover:hover{border-bottom-style:var(--sdui--border-bottom-style--hover)!important}[_nghost-%COMP%]     .sdui--border-bottom-style--focus:focus-visible{border-bottom-style:var(--sdui--border-bottom-style--focus)!important}[_nghost-%COMP%]     .sdui--border-bottom-width{border-bottom-width:var(--sdui--border-bottom-width)!important}[_nghost-%COMP%]     .sdui--border-bottom-width--hover:hover{border-bottom-width:var(--sdui--border-bottom-width--hover)!important}[_nghost-%COMP%]     .sdui--border-bottom-width--focus:focus-visible{border-bottom-width:var(--sdui--border-bottom-width--focus)!important}[_nghost-%COMP%]     .sdui--border-left-color{border-left-color:var(--sdui--border-left-color)!important}[_nghost-%COMP%]     .sdui--border-left-color--hover:hover{border-left-color:var(--sdui--border-left-color--hover)!important}[_nghost-%COMP%]     .sdui--border-left-color--focus:focus-visible{border-left-color:var(--sdui--border-left-color--focus)!important}[_nghost-%COMP%]     .sdui--border-left-style{border-left-style:var(--sdui--border-left-style)!important}[_nghost-%COMP%]     .sdui--border-left-style--hover:hover{border-left-style:var(--sdui--border-left-style--hover)!important}[_nghost-%COMP%]     .sdui--border-left-style--focus:focus-visible{border-left-style:var(--sdui--border-left-style--focus)!important}[_nghost-%COMP%]     .sdui--border-left-width{border-left-width:var(--sdui--border-left-width)!important}[_nghost-%COMP%]     .sdui--border-left-width--hover:hover{border-left-width:var(--sdui--border-left-width--hover)!important}[_nghost-%COMP%]     .sdui--border-left-width--focus:focus-visible{border-left-width:var(--sdui--border-left-width--focus)!important}[_nghost-%COMP%]     .sdui--border-right-color{border-right-color:var(--sdui--border-right-color)!important}[_nghost-%COMP%]     .sdui--border-right-color--hover:hover{border-right-color:var(--sdui--border-right-color--hover)!important}[_nghost-%COMP%]     .sdui--border-right-color--focus:focus-visible{border-right-color:var(--sdui--border-right-color--focus)!important}[_nghost-%COMP%]     .sdui--border-right-style{border-right-style:var(--sdui--border-right-style)!important}[_nghost-%COMP%]     .sdui--border-right-style--hover:hover{border-right-style:var(--sdui--border-right-style--hover)!important}[_nghost-%COMP%]     .sdui--border-right-style--focus:focus-visible{border-right-style:var(--sdui--border-right-style--focus)!important}[_nghost-%COMP%]     .sdui--border-right-width{border-right-width:var(--sdui--border-right-width)!important}[_nghost-%COMP%]     .sdui--border-right-width--hover:hover{border-right-width:var(--sdui--border-right-width--hover)!important}[_nghost-%COMP%]     .sdui--border-right-width--focus:focus-visible{border-right-width:var(--sdui--border-right-width--focus)!important}[_nghost-%COMP%]     .sdui--border-top-color{border-top-color:var(--sdui--border-top-color)!important}[_nghost-%COMP%]     .sdui--border-top-color--hover:hover{border-top-color:var(--sdui--border-top-color--hover)!important}[_nghost-%COMP%]     .sdui--border-top-color--focus:focus-visible{border-top-color:var(--sdui--border-top-color--focus)!important}[_nghost-%COMP%]     .sdui--border-top-style{border-top-style:var(--sdui--border-top-style)!important}[_nghost-%COMP%]     .sdui--border-top-style--hover:hover{border-top-style:var(--sdui--border-top-style--hover)!important}[_nghost-%COMP%]     .sdui--border-top-style--focus:focus-visible{border-top-style:var(--sdui--border-top-style--focus)!important}[_nghost-%COMP%]     .sdui--border-top-width{border-top-width:var(--sdui--border-top-width)!important}[_nghost-%COMP%]     .sdui--border-top-width--hover:hover{border-top-width:var(--sdui--border-top-width--hover)!important}[_nghost-%COMP%]     .sdui--border-top-width--focus:focus-visible{border-top-width:var(--sdui--border-top-width--focus)!important}[_nghost-%COMP%]     .sdui--box-shadow{box-shadow:var(--sdui--box-shadow)!important}[_nghost-%COMP%]     .sdui--box-shadow--hover:hover{box-shadow:var(--sdui--box-shadow--hover)!important}[_nghost-%COMP%]     .sdui--box-shadow--focus:focus-visible{box-shadow:var(--sdui--box-shadow--focus)!important}[_nghost-%COMP%]     .sdui--color{color:var(--sdui--color)!important}[_nghost-%COMP%]     .sdui--color--hover:hover{color:var(--sdui--color--hover)!important}[_nghost-%COMP%]     .sdui--color--focus:focus-visible{color:var(--sdui--color--focus)!important}[_nghost-%COMP%]     .sdui--cursor{cursor:var(--sdui--cursor)!important}[_nghost-%COMP%]     .sdui--cursor--hover:hover{cursor:var(--sdui--cursor--hover)!important}[_nghost-%COMP%]     .sdui--cursor--focus:focus-visible{cursor:var(--sdui--cursor--focus)!important}[_nghost-%COMP%]     .sdui--outline-color{outline-color:var(--sdui--outline-color)!important}[_nghost-%COMP%]     .sdui--outline-color--hover:hover{outline-color:var(--sdui--outline-color--hover)!important}[_nghost-%COMP%]     .sdui--outline-color--focus:focus-visible{outline-color:var(--sdui--outline-color--focus)!important}[_nghost-%COMP%]     .sdui--outline-style{outline-style:var(--sdui--outline-style)!important}[_nghost-%COMP%]     .sdui--outline-style--hover:hover{outline-style:var(--sdui--outline-style--hover)!important}[_nghost-%COMP%]     .sdui--outline-style--focus:focus-visible{outline-style:var(--sdui--outline-style--focus)!important}[_nghost-%COMP%]     .sdui--outline-width{outline-width:var(--sdui--outline-width)!important}[_nghost-%COMP%]     .sdui--outline-width--hover:hover{outline-width:var(--sdui--outline-width--hover)!important}[_nghost-%COMP%]     .sdui--outline-width--focus:focus-visible{outline-width:var(--sdui--outline-width--focus)!important}[_nghost-%COMP%]     .sdui--text-shadow{text-shadow:var(--sdui--text-shadow)!important}[_nghost-%COMP%]     .sdui--text-shadow--hover:hover{text-shadow:var(--sdui--text-shadow--hover)!important}[_nghost-%COMP%]     .sdui--text-shadow--focus:focus-visible{text-shadow:var(--sdui--text-shadow--focus)!important}  .sdui-hidden{display:none!important}"]
		});
		_.yKc = new _.he("MenuServiceBase");
		zKc = function(a) {
			if (a & 1) {
				_.Ih(0, 1);
			}
			if (a & 2) {
				a = _.K(), _.E("comp", a.tooltip().pR());
			}
		};
		AKc = function(a) {
			if (a & 1) {
				_.F(0, "div", 2), _.Ih(1, 1), _.H();
			}
			if (a & 2) {
				a = _.K(2), _.y(), a = a.tooltip(), a = _.Z(a, _.tY, 3), _.E("comp", a);
			}
		};
		BKc = function(a) {
			if (a & 1) {
				_.z(0, AKc, 2, 1, "ng-template", null, 0, _.Ii);
			}
		};
		CKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		DKc = function(a) {
			if (a & 1) {
				_.z(0, CKc, 1, 0, "ng-container", 3);
			}
			if (a & 2) {
				_.K(), a = _.O(5), _.E("ngTemplateOutlet", a);
			}
		};
		EKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		FKc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 6, 2);
				_.J("click", function(c) {
					_.q(b);
					var d = _.K(3);
					return _.t(d.zR(c));
				});
				_.z(2, EKc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				let b;
				a = _.K(3);
				let c = _.O(5);
				_.E("ngClass", a.MA())("color", a.color())("disabled", !!a.disabled())("disabledTooltip", a.sL())("disabledTooltipRich", a.ds())("cfcTooltip", a.Pv())("cfcTooltipDisabled", !!a.disabled())("cfcTooltipInteractive", a.ds())("cfcTooltipInteractiveDialogAriaLabel", a.oO())("cfcTooltipAriaRelationship", a.pO());
				_.wh("href", (b = a.link()) == null ? null : b.url, _.rg);
				_.y(2);
				_.E("ngTemplateOutlet", c);
			}
		};
		GKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		HKc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 7, 2);
				_.J("click", function() {
					_.q(b);
					var c = _.K(3);
					return _.t(c.aM());
				});
				_.z(2, GKc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				a = _.K(3);
				let b = _.O(5);
				_.E("ngClass", a.MA())("type", a.type())("color", a.color())("disabled", !!a.disabled())("disabledTooltip", a.sL())("disabledTooltipRich", a.ds())("cfcTooltip", a.Pv())("cfcTooltipDisabled", !!a.disabled())("cfcTooltipInteractive", a.ds())("cfcTooltipInteractiveDialogAriaLabel", a.oO())("cfcTooltipAriaRelationship", a.pO());
				_.y(2);
				_.E("ngTemplateOutlet", b);
			}
		};
		IKc = function(a) {
			if (a & 1) {
				_.B(0, FKc, 3, 12, "a", 4)(1, HKc, 3, 12, "button", 5);
			}
			if (a & 2) {
				a = _.K(2), _.C(a.Kz() ? 0 : 1);
			}
		};
		JKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		KKc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 10, 2);
				_.J("click", function(c) {
					_.q(b);
					var d = _.K(3);
					return _.t(d.zR(c));
				});
				_.z(2, JKc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				let b;
				a = _.K(3);
				let c = _.O(5);
				_.E("ngClass", a.MA())("disabled", !!a.disabled())("disabledTooltip", a.sL())("disabledTooltipRich", a.ds())("cfcTooltip", a.Pv())("cfcTooltipDisabled", !!a.disabled())("cfcTooltipInteractive", a.ds())("cfcTooltipInteractiveDialogAriaLabel", a.oO())("cfcTooltipAriaRelationship", a.pO());
				_.wh("href", (b = a.link()) == null ? null : b.url, _.rg);
				_.y(2);
				_.E("ngTemplateOutlet", c);
			}
		};
		LKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		MKc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 11, 2);
				_.J("click", function() {
					_.q(b);
					var c = _.K(3);
					return _.t(c.aM());
				});
				_.z(2, LKc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				a = _.K(3);
				let b = _.O(5);
				_.E("ngClass", a.MA())("type", a.type())("disabled", !!a.disabled())("disabledTooltip", a.sL())("disabledTooltipRich", a.ds())("cfcTooltip", a.Pv())("cfcTooltipDisabled", !!a.disabled())("cfcTooltipInteractive", a.ds())("cfcTooltipInteractiveDialogAriaLabel", a.oO())("cfcTooltipAriaRelationship", a.pO());
				_.y(2);
				_.E("ngTemplateOutlet", b);
			}
		};
		NKc = function(a) {
			if (a & 1) {
				_.B(0, KKc, 3, 11, "a", 8)(1, MKc, 3, 11, "button", 9);
			}
			if (a & 2) {
				a = _.K(2), _.C(a.Kz() ? 0 : 1);
			}
		};
		OKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		PKc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 14, 2);
				_.J("click", function(c) {
					_.q(b);
					var d = _.K(3);
					return _.t(d.zR(c));
				});
				_.z(2, OKc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				let b;
				a = _.K(3);
				let c = _.O(5);
				_.E("ngClass", a.MA())("color", a.color())("disabled", !!a.disabled())("disabledTooltip", a.sL())("disabledTooltipRich", a.ds())("cfcTooltip", a.Pv())("cfcTooltipDisabled", !!a.disabled())("cfcTooltipInteractive", a.ds())("cfcTooltipInteractiveDialogAriaLabel", a.oO())("cfcTooltipAriaRelationship", a.pO());
				_.wh("href", (b = a.link()) == null ? null : b.url, _.rg);
				_.y(2);
				_.E("ngTemplateOutlet", c);
			}
		};
		QKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		RKc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 15, 2);
				_.J("click", function() {
					_.q(b);
					var c = _.K(3);
					return _.t(c.aM());
				});
				_.z(2, QKc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				a = _.K(3);
				let b = _.O(5);
				_.E("ngClass", a.MA())("type", a.type())("color", a.color())("disabled", !!a.disabled())("disabledTooltip", a.sL())("disabledTooltipRich", a.ds())("cfcTooltip", a.Pv())("cfcTooltipDisabled", !!a.disabled())("cfcTooltipInteractive", a.ds())("cfcTooltipInteractiveDialogAriaLabel", a.oO())("cfcTooltipAriaRelationship", a.pO());
				_.y(2);
				_.E("ngTemplateOutlet", b);
			}
		};
		SKc = function(a) {
			if (a & 1) {
				_.B(0, PKc, 3, 12, "a", 12)(1, RKc, 3, 12, "button", 13);
			}
			if (a & 2) {
				a = _.K(2), _.C(a.Kz() ? 0 : 1);
			}
		};
		TKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		UKc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 18, 2);
				_.J("click", function(c) {
					_.q(b);
					var d = _.K(3);
					return _.t(d.zR(c));
				});
				_.z(2, TKc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				let b;
				a = _.K(3);
				let c = _.O(5);
				_.wh("href", (b = a.link()) == null ? null : b.url, _.rg);
				_.y(2);
				_.E("ngTemplateOutlet", c);
			}
		};
		VKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		WKc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 19, 2);
				_.J("click", function() {
					_.q(b);
					var c = _.K(3);
					return _.t(c.aM());
				});
				_.z(2, VKc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				a = _.K(3);
				let b = _.O(5);
				_.E("type", a.type());
				_.y(2);
				_.E("ngTemplateOutlet", b);
			}
		};
		XKc = function(a) {
			if (a & 1) {
				_.B(0, UKc, 3, 2, "a", 16)(1, WKc, 3, 2, "button", 17);
			}
			if (a & 2) {
				a = _.K(2), _.C(a.Kz() ? 0 : 1);
			}
		};
		YKc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		ZKc = function(a) {
			if (a & 1) {
				_.z(0, YKc, 1, 0, "ng-container", 3);
			}
			if (a & 2) {
				_.K(2), a = _.O(3), _.E("ngTemplateOutlet", a);
			}
		};
		$Kc = function(a) {
			if (a & 1) {
				_.B(0, IKc, 2, 1)(1, NKc, 2, 1)(2, SKc, 2, 1)(3, XKc, 2, 1)(4, ZKc, 1, 1, "ng-container");
			}
			if (a & 2) {
				let b;
				a = _.K();
				_.C((b = a.kF()) === "raised" ? 0 : b === "flat" ? 1 : b === "stroked" ? 2 : b === "unstyled" ? 3 : 4);
			}
		};
		aLc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		bLc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "a", 22, 2);
				_.J("click", function(c) {
					_.q(b);
					var d = _.K(2);
					return _.t(d.zR(c));
				});
				_.z(2, aLc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				let b;
				a = _.K(2);
				let c = _.O(5);
				_.E("ngClass", a.MA())("color", a.color())("disabled", !!a.disabled())("disabledTooltip", a.sL())("disabledTooltipRich", a.ds())("cfcTooltip", a.Pv())("cfcTooltipDisabled", !!a.disabled())("cfcTooltipInteractive", a.ds())("cfcTooltipInteractiveDialogAriaLabel", a.oO())("cfcTooltipAriaRelationship", a.pO());
				_.wh("href", (b = a.link()) == null ? null : b.url, _.rg);
				_.y(2);
				_.E("ngTemplateOutlet", c);
			}
		};
		cLc = function(a) {
			if (a & 1) {
				_.Ih(0);
			}
		};
		dLc = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "button", 23, 2);
				_.J("click", function() {
					_.q(b);
					var c = _.K(2);
					return _.t(c.aM());
				});
				_.z(2, cLc, 1, 0, "ng-container", 3);
				_.H();
			}
			if (a & 2) {
				a = _.K(2);
				let b = _.O(5);
				_.E("ngClass", a.MA())("type", a.type())("color", a.color())("disabled", !!a.disabled())("disabledTooltip", a.sL())("disabledTooltipRich", a.ds())("cfcTooltip", a.Pv())("cfcTooltipDisabled", !!a.disabled())("cfcTooltipInteractive", a.ds())("cfcTooltipInteractiveDialogAriaLabel", a.oO())("cfcTooltipAriaRelationship", a.pO());
				_.y(2);
				_.E("ngTemplateOutlet", b);
			}
		};
		eLc = function(a) {
			if (a & 1) {
				_.B(0, bLc, 3, 12, "a", 20)(1, dLc, 3, 12, "button", 21);
			}
			if (a & 2) {
				a = _.K(), _.C(a.Kz() ? 0 : 1);
			}
		};
		fLc = function(a) {
			if (a & 1) {
				_.F(0, "div", 25), _.Ih(1, 26), _.R(2, " "), _.I(3, "cm-icon", 27), _.H();
			}
			if (a & 2) {
				a = _.K(3), _.y(), _.E("comp", a.button().Sb()), _.y(2), _.E("icon", a.yza), _.wh("aria-label", a.bta);
			}
		};
		gLc = function(a) {
			if (a & 1) {
				_.Ih(0, 26);
			}
			if (a & 2) {
				a = _.K(3), _.E("comp", a.button().Sb());
			}
		};
		hLc = function(a) {
			if (a & 1) {
				_.F(0, "div", 24), _.B(1, fLc, 4, 3, "div", 25)(2, gLc, 1, 1, "ng-container", 26), _.H();
			}
			if (a & 2) {
				a = _.K(2), _.y(), _.C(a.Kba() ? 1 : 2);
			}
		};
		iLc = function(a) {
			if (a & 1) {
				_.B(0, hLc, 3, 1, "div", 24);
			}
			if (a & 2) {
				a = _.K(), _.C(a.button().Fe() ? 0 : -1);
			}
		};
		jLc = function(a) {
			return a == null || a.hasImage() || _.Dr(a, _.RFc, 1e3, _.CY) && _.TFc(a).QL().every((b) => jLc(b));
		};
		_.NZ = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.NZ.prototype.NY = _.ba(197);
		_.tY.prototype.Mja = _.ca(202, function() {
			return _.Dr(this, _.QFc, 1001, _.CY);
		});
		_.NZ.prototype.NY = _.ca(197, function() {
			return _.l(this, 1);
		});
		_.YEc.prototype.NY = _.ca(196, function() {
			return _.l(this, 1);
		});
		_.kLc = function(a) {
			var b = new _.QX();
			return _.Ap(b, 4, _.mY, a);
		};
		OZ = class {
			constructor() {
				var a = _.m(_.BQ);
				if (_.m(_.N7b, { optional: true })) {
					var b = a.R;
					_.Kg(() => {
						b();
					});
				}
			}
		};
		OZ.J = function(a) {
			return new (a || OZ)();
		};
		OZ.Oa = _.We({
			type: OZ,
			da: [
				[
					"button",
					"mat-fab",
					""
				],
				[
					"a",
					"mat-fab",
					""
				],
				[
					"button",
					"mat-mini-fab",
					""
				],
				[
					"a",
					"mat-mini-fab",
					""
				],
				[
					"button",
					"mat-raised-button",
					""
				],
				[
					"a",
					"mat-raised-button",
					""
				]
			],
			standalone: false
		});
		mLc = new _.he("GM2_DIALOG_OPTIONS", {
			wa: "root",
			factory: () => ({ yl: false })
		});
		_.PZ = class extends _.rC {
			constructor() {
				super(...arguments);
				this.Aa = _.m(mLc);
				this.defaultOptions = _.m(_.Djb, { optional: true });
				this.X = ["gmat-mdc-dialog"];
			}
			open(a, b) {
				b = Object.assign({}, this.defaultOptions || new _.Cjb(), b);
				if (this.Aa.yl) throw Error("Wb");
				b.Rc = _.Ll(b.Rc || []).concat(this.X);
				return super.open(a, b);
			}
		};
		_.PZ.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(_.PZ)))(b || _.PZ);
			};
		})();
		_.PZ.sa = _.Cd({
			token: _.PZ,
			factory: _.PZ.J,
			wa: "root"
		});
		nLc = ["richContentTemplate"];
		_.QZ = class {
			constructor() {
				this.xb = _.Li.required();
				this.id = _.W(() => this.xb().getId() || _.iu());
				this.tooltip = _.W(() => {
					var a = this.xb();
					return _.fj(a, _.tFc, 1006, _.CY);
				});
				this.ariaLabel = _.W(() => {
					var a;
					return (a = _.BY(this.xb())) == null ? undefined : a.Bl();
				});
				this.Lg = this.oj = this.A = _.W(() => {});
				this.content = _.W(() => {
					var a = this.tooltip();
					return _.l(a, 2) || this.d9a();
				});
				this.d9a = _.Ni("richContentTemplate");
			}
		};
		_.QZ.J = function(a) {
			return new (a || _.QZ)();
		};
		_.QZ.ka = _.u({
			type: _.QZ,
			da: [["sdui-tooltip"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.d9a, nLc, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			Ua: 2,
			Ja: function(a) {
				if (a & 2) {
					_.pi("display", "contents");
				}
			},
			inputs: { xb: [1, "comp"] },
			features: [_.yi([{
				Da: _.kP,
				zb: _.QZ
			}])],
			ha: 2,
			ia: 2,
			la: [
				["richContentTemplate", ""],
				[
					"dynamic-sdui-component",
					"",
					3,
					"comp"
				],
				["sdui-token-host", ""]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, zKc, 1, 1, "ng-container", 1), _.B(1, BKc, 2, 0);
				}
				if (a & 2) {
					let c;
					_.C(((c = b.tooltip().pR()) == null ? 0 : c.Mja()) ? 0 : -1);
					_.y();
					a = b.tooltip();
					a = _.sn(a, _.tY, 3);
					_.C(a ? 1 : -1);
				}
			},
			dependencies: [
				_.tz,
				_.AZ,
				_.cR
			],
			Ab: 2
		});
		oLc = function(a, b, c) {
			if (!a.forms.has(b)) {
				a.forms.set(b, {
					components: new Map(),
					SU: undefined
				});
			}
			var d = a.forms.get(b);
			d.SU = c;
			return () => {
				if (d.SU === c) {
					d.SU = undefined, d.components.size === 0 && a.forms.delete(b);
				}
			};
		};
		_.RZ = class {
			constructor() {
				this.forms = new Map();
			}
			submit(a) {
				a = this.forms.get(a);
				if (a.SU) {
					a.SU();
				}
			}
		};
		_.RZ.J = function(a) {
			return new (a || _.RZ)();
		};
		_.RZ.sa = _.Cd({
			token: _.RZ,
			factory: _.RZ.J
		});
		pLc = ["anchorOrButtonElement"];
		qLc = function(a, b, c) {
			var d = a.A();
			if (!b || !d) return null;
			a.U.navigate(b, c);
			d.ze();
			return null;
		};
		_.SZ = class {
			constructor() {
				this.QMb = true;
				this.xb = _.Li.required();
				this.id = _.W(() => this.xb().getId() || _.iu());
				this.button = _.W(() => this.xb().OL());
				this.I = _.W(() => _.SFc(this.xb()));
				this.aa = _.m(_.vZ);
				this.Lg = this.oj = this.EP = _.Ni("anchorOrButtonElement", Object.assign({}, {}, { read: _.Jf }));
				this.JB = _.W(() => this.kF() !== "unstyled" && jLc(this.button().Sb()));
				this.link = _.M(null);
				this.Kba = _.W(() => {
					if (!this.Kz()) return false;
					var c;
					return ((c = this.link()) == null ? undefined : c.type) !== "EXTERNAL" || this.JB() || this.kF() === "unstyled" ? false : true;
				});
				this.yza = _.k4b;
				this.bta = "external, opens new window";
				this.Kz = _.W(() => this.A() != null && this.X().length === 0);
				this.kF = _.W(() => {
					var c = this.button().mE();
					switch (c) {
						case 2: return "raised";
						case 3:
						case 11:
						case 10: return "flat";
						case 9:
						case 4: return "stroked";
						case 6: return "unstyled";
						case 1:
						case 5:
						case 7:
						case 8:
						case 0: return "basic";
						default: _.sb(c, undefined);
					}
				});
				this.color = _.W(() => {
					if (this.button().ig() === 2) return "primary";
					var c = this.button().mE();
					return c === 8 || c === 9 || c === 10 || c === 11 ? "primary" : null;
				});
				this.R = _.W(() => {
					var c = this.button();
					return _.l(c, 9);
				});
				this.type = _.W(() => this.R() ? "submit" : "button");
				this.MA = _.W(() => {
					var c = this.button().getSize();
					var d = this.JB();
					return this.kF() === "unstyled" ? {} : {
						"sdui-button": true,
						"sdui-icon-button": d,
						"button-small": c === 1,
						"button-medium": c === 2 || c === 3 || c === 0
					};
				});
				this.disabled = _.W(() => {
					var c = _.AY(this.xb());
					var d = c.has("button.disabled") ? "button.disabled" : "disabled";
					return this.aa.evaluate(d, "bool", this.button().Wy(), c) ? true : null;
				});
				this.sL = _.W(() => {
					var c;
					return ((c = this.I()) == null ? undefined : _.l(c, 2)) || this.Pv() || "Currently disabled.";
				});
				this.ds = _.W(() => this.Pv() instanceof _.Zh);
				this.pO = _.W(() => {
					var c;
					var d = (c = _.BY(this.xb())) == null ? undefined : c.hasLabel();
					return this.JB() ? d ? "none" : "label" : "description";
				});
				this.Pv = _.W(() => {
					var c;
					var d;
					return ((c = this.I()) == null ? undefined : c.getText()) || ((d = this.H) == null ? undefined : d.content());
				});
				this.oO = _.W(() => {
					var c;
					return (c = this.H) == null ? undefined : c.ariaLabel();
				});
				this.ea = _.W(() => {
					var c;
					return ((c = this.button().zo().find((d) => d == null ? undefined : _.Dr(d, _.EY, 12, _.FY))) == null ? undefined : _.fj(c, _.EY, 12, _.FY)) || null;
				});
				this.A = _.W(() => {
					var c;
					return ((c = this.button().zo().find((d) => d == null ? undefined : _.HY(d))) == null ? undefined : _.GY(c)) || null;
				});
				this.fa = _.W(() => this.button().zo().filter((c) => !!c && _.IY(c)));
				this.X = _.W(() => this.button().zo().filter((c) => !!c && !_.IY(c) && !_.HY(c)));
				this.H = _.m(_.QZ, { optional: true });
				this.F = _.m(_.kZ);
				this.U = _.m(_.DZ);
				_.m(_.EZ);
				this.bf = _.m(_.$h);
				var a = _.m(_.yKc);
				var b = _.m(_.RZ);
				_.Kg((c) => {
					var d = this.R();
					if (d) {
						d = oLc(b, d, () => {
							if (!this.disabled()) {
								this.aM();
							}
						}), c(d);
					}
				});
				_.Kg(() => {
					var c = this.A();
					if (this.Kz() && c && _.zn(c, 1)) {
						c.ze() ? this.link.set(null) : this.link.set(_.pKc(c.getUrl()));
					}
				});
				_.cj({ rma: () => {
					var c;
					var d = (c = this.Lg()) == null ? undefined : c.nativeElement;
					if (d && d.hasAttribute("aria-label") && this.Kba()) {
						c = d.getAttribute("aria-label"), d.setAttribute("aria-label", `${c} external, opens new window`);
					}
				} });
				_.cj({ read: (c) => {
					var d;
					var e = (d = this.Lg()) == null ? undefined : d.nativeElement;
					var f = this.ea();
					if (f && e) {
						_.Qd(() => {
							a.q8a(f, e);
							c(() => {
								a.lYa(f);
							});
						});
					}
				} });
			}
			zR(a) {
				a.preventDefault();
				var b = this.link();
				this.fa().forEach((c) => {
					this.F.Zg(c, this.bf);
				});
				a = qLc(this, b, { Sq: !(!a.ctrlKey && !a.metaKey) });
				this.link.set(a || b);
			}
			aM() {
				for (let a of this.button().zo()) this.F.Zg(a, this.bf);
			}
		};
		_.SZ.J = function(a) {
			return new (a || _.SZ)();
		};
		_.SZ.ka = _.u({
			type: _.SZ,
			da: [["sdui-button"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.EP, pLc, 5, _.Jf);
				}
				if (a & 2) {
					_.ki();
				}
			},
			Ua: 2,
			Ja: function(a) {
				if (a & 2) {
					_.pi("display", "contents");
				}
			},
			inputs: { xb: [1, "comp"] },
			features: [_.yi([{
				Da: _.kP,
				zb: _.SZ
			}])],
			ha: 6,
			ia: 1,
			la: [
				["basic", ""],
				["content", ""],
				["anchorOrButtonElement", ""],
				[4, "ngTemplateOutlet"],
				[
					"mat-raised-button",
					"",
					3,
					"ngClass",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-raised-button",
					"",
					3,
					"ngClass",
					"type",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-raised-button",
					"",
					3,
					"click",
					"ngClass",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-raised-button",
					"",
					3,
					"click",
					"ngClass",
					"type",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-flat-button",
					"",
					"color",
					"primary",
					3,
					"ngClass",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-flat-button",
					"",
					"color",
					"primary",
					3,
					"ngClass",
					"type",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-flat-button",
					"",
					"color",
					"primary",
					3,
					"click",
					"ngClass",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-flat-button",
					"",
					"color",
					"primary",
					3,
					"click",
					"ngClass",
					"type",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-stroked-button",
					"",
					3,
					"ngClass",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-stroked-button",
					"",
					3,
					"ngClass",
					"type",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-stroked-button",
					"",
					3,
					"click",
					"ngClass",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-stroked-button",
					"",
					3,
					"click",
					"ngClass",
					"type",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[1, "anchor-unstyled"],
				[
					1,
					"button-unstyled",
					3,
					"type"
				],
				[
					1,
					"anchor-unstyled",
					3,
					"click"
				],
				[
					1,
					"button-unstyled",
					3,
					"click",
					"type"
				],
				[
					"mat-button",
					"",
					3,
					"ngClass",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-button",
					"",
					3,
					"ngClass",
					"type",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-button",
					"",
					3,
					"click",
					"ngClass",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[
					"mat-button",
					"",
					3,
					"click",
					"ngClass",
					"type",
					"color",
					"disabled",
					"disabledTooltip",
					"disabledTooltipRich",
					"cfcTooltip",
					"cfcTooltipDisabled",
					"cfcTooltipInteractive",
					"cfcTooltipInteractiveDialogAriaLabel",
					"cfcTooltipAriaRelationship"
				],
				[1, "sdui-button-content"],
				[1, "button-external-link-container"],
				[
					"dynamic-sdui-component",
					"",
					3,
					"comp"
				],
				[
					"role",
					"img",
					1,
					"button-external-link-icon",
					3,
					"icon"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, DKc, 1, 1, "ng-container")(1, $Kc, 5, 1), _.z(2, eLc, 2, 1, "ng-template", null, 0, _.Ii)(4, iLc, 1, 1, "ng-template", null, 1, _.Ii);
				}
				if (a & 2) {
					_.C(b.Kz() && b.link() === null ? 0 : 1);
				}
			},
			dependencies: [
				_.$Q,
				_.UC,
				_.XB,
				_.ZQ,
				OZ,
				_.SP,
				_.kz,
				_.nz,
				_.bR,
				_.aR,
				_.AZ
			],
			styles: [".sdui-button.sdui-button.sdui-button[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;margin:0}.sdui-button.sdui-button.sdui-button[_ngcontent-%COMP%]     img, .sdui-button.sdui-button.sdui-button[_ngcontent-%COMP%]     svg:not(cm-icon>*){height:18px;width:18px}.sdui-button-content[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:1;-webkit-flex-grow:1;-moz-box-flex:1;-ms-flex-positive:1;flex-grow:1}.sdui-button[_ngcontent-%COMP%]     sdui-block{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}.sdui-button[_ngcontent-%COMP%]:not(.sdui-icon-button)     img, .sdui-button[_ngcontent-%COMP%]:not(.sdui-icon-button)     svg:not(cm-icon>*){margin-right:2px;margin-top:calc(-.356em - -.5ex)}.button-unstyled[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;border:none;background:none;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;font:inherit;padding:0;position:relative}.button-unstyled[_ngcontent-%COMP%]:focus-visible{outline:3px solid var(--sdui-sys-color-primary,#0c67df);outline-offset:2px}.anchor-unstyled[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;border:none;color:inherit;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}.button-small[_ngcontent-%COMP%]{--cm-md1-button-line-height:24px;--cm-md1-button-padding-inline:8px;--cm-icon-button-height:24px;--cm-icon-button-width:24px;--cm-icon-button-line-height:24px;--cm-gm2-button-height:28px}.button-small[_ngcontent-%COMP%]   .cfc-progress-button-unresolved[_ngcontent-%COMP%], .button-small[_ngcontent-%COMP%]   .mat-button-toggle-label-content[_ngcontent-%COMP%], .button-small[_ngcontent-%COMP%]   [mat-button][_ngcontent-%COMP%], .button-small[_ngcontent-%COMP%]   [mat-flat-button][_ngcontent-%COMP%], .button-small[_ngcontent-%COMP%]   [mat-raised-button][_ngcontent-%COMP%], .button-small[_ngcontent-%COMP%]   [mat-stroked-button][_ngcontent-%COMP%], .button-small.mat-button-toggle-label-content[_ngcontent-%COMP%], .button-small[mat-button][_ngcontent-%COMP%], .button-small[mat-flat-button][_ngcontent-%COMP%], .button-small[mat-raised-button][_ngcontent-%COMP%], .button-small[mat-stroked-button][_ngcontent-%COMP%]{line-height:24px;padding:0 8px}.button-small[_ngcontent-%COMP%]   [mat-icon-button][_ngcontent-%COMP%], .button-small[mat-icon-button][_ngcontent-%COMP%]{--cm-focus-indicator-offset:2px;padding:0}.button-small[_ngcontent-%COMP%]   [mat-icon-button][_ngcontent-%COMP%]:where(:not(.cm-button)), .button-small[mat-icon-button][_ngcontent-%COMP%]:where(:not(.cm-button)){height:24px;line-height:24px;width:24px}.button-small[_ngcontent-%COMP%]   .cfc-split-button-menu-button[_ngcontent-%COMP%], .button-small[_ngcontent-%COMP%]   .cfc-split-button-menu-button.cm-button[_ngcontent-%COMP%], .button-small.cfc-split-button-menu-button[_ngcontent-%COMP%], .button-small.cfc-split-button-menu-button.cm-button[_ngcontent-%COMP%]{padding:0}.button-small[_ngcontent-%COMP%]   .cfc-split-button[_ngcontent-%COMP%], .button-small.cfc-split-button[_ngcontent-%COMP%]{line-height:20px}body[_ngcontent-%COMP%]   .mat-icon-button.mat-icon-button[_ngcontent-%COMP%]:before, body.mat-icon-button.mat-icon-button[_ngcontent-%COMP%]:before{margin:-2px}.button-medium[_ngcontent-%COMP%]{--cm-icon-button-height:32px;--cm-icon-button-width:32px;--cm-icon-button-line-height:32px}.button-medium[_ngcontent-%COMP%]   [mat-icon-button][_ngcontent-%COMP%], .button-medium[mat-icon-button][_ngcontent-%COMP%]{padding:0}.button-medium[_ngcontent-%COMP%]   [mat-icon-button][_ngcontent-%COMP%]:where(:not(.cm-button)), .button-medium[mat-icon-button][_ngcontent-%COMP%]:where(:not(.cm-button)){height:32px;line-height:32px;width:32px}.sdui-icon-button[_ngcontent-%COMP%]{--mat-focus-indicator-border-radius:50%;--mat-focus-indicator-border-color:var(--mat-sys-secondary,#000);--mat-focus-indicator-display:block;border-radius:50%}.sdui-icon-button[_ngcontent-%COMP%]   .mat-mdc-standard-chip[_ngcontent-%COMP%]   .mat-mdc-chip-action-label[_ngcontent-%COMP%], .sdui-icon-button[_ngcontent-%COMP%]   .mat-mdc-standard-chip[_ngcontent-%COMP%]   .mdc-evolution-chip__action--primary[_ngcontent-%COMP%], .sdui-icon-button[_ngcontent-%COMP%]   .mat-mdc-standard-chip[_ngcontent-%COMP%]   .mdc-evolution-chip__cell--primary[_ngcontent-%COMP%]{overflow:visible}.sdui-icon-button.button-small[_ngcontent-%COMP%]{width:24px;height:24px}.sdui-icon-button.button-medium[_ngcontent-%COMP%]{width:40px;height:40px}.button-external-link-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}.button-external-link-icon.button-external-link-icon.button-external-link-icon.button-external-link-icon[_ngcontent-%COMP%]{margin:0;overflow:hidden}"]
		});
		_.rLc = new _.he("SDUI_VIEWER_INJECTION_TOKEN");
		_.R_ = class extends _.h {
			constructor(a) {
				super(a);
			}
			D7() {
				return _.l(this, 1);
			}
			Cl() {
				return _.yj(this, 3);
			}
		};
		_.rSc = function(a, b) {
			return _.Uc(a, 1, b);
		};
		_.S_ = function(a) {
			return _.Z(a, _.R_, 2);
		};
		_.sSc = function(a) {
			var b = new _.Gbb();
			return _.ln(b, _.oy, 1, a);
		};
		_.tSc = function(a, b) {
			return _.$q(a.A, a.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/UpdateProjectUsageLimit", b, {}, _.Hbb);
		};
		_.uSc = function(a, b) {
			var c = a.Pn();
			var d;
			var e = _.rSc(new _.oy(), (d = c == null ? undefined : c.getName()) != null ? d : b.getName());
			if (_.sn(b, _.R_, 2)) {
				b = _.S_(b), _.ln(e, _.R_, 2, b);
			} else {
				_.In(e, 2);
			}
			if (c == null ? 0 : _.sn(c, _.R_, 3)) {
				b = _.Z(c, _.R_, 3);
				_.ln(e, _.R_, 3, b);
			}
			if (c == null ? 0 : _.sn(c, _.R_, 4)) {
				c = _.Z(c, _.R_, 4);
				_.ln(e, _.R_, 4, c);
			}
			a.resource.set(e);
		};
		_.T_ = class {
			constructor() {
				this.H = _.m(_.Zq);
				this.F = _.m(_.Ou);
				this.I = _.m(_.Op);
				this.project = _.M();
				this.A = _.M(false);
				this.PB = this.A.asReadonly();
				this.resource = _.Zi(Object.assign({}, {}, {
					params: this.project,
					Xc: ({ params: a }) => {
						var b = this;
						return _.x(function* () {
							var c;
							if (c = a) {
								if (b.I.getFlag(_.VFb)) if (_.Pm(a, 13)) var d = true;
								else {
									c = (d = _.au(a)) == null ? undefined : _.Lm(d, 4);
									d = c === 5 || c === 3;
								}
								else d = false;
								c = !d && (a ? a.Ap().includes(16) : false);
							}
							if (c) return d = new _.h8a(), c = a.getName(), d = _.Uc(d, 1, c), c = b.H, yield _.$q(c.A, c.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/GetProjectUsageLimit", d, {}, _.j8a);
						});
					}
				}));
				this.Pn = _.W(() => this.resource.xc() ? this.resource.value() : undefined);
				this.R = _.W(() => _.pn(this.resource.error()));
				this.Sa = this.resource.Sa;
			}
			update(a) {
				var b = this;
				return _.x(function* () {
					b.A.set(true);
					var c = _.sSc(a);
					try {
						let d = yield _.tSc(b.H, c);
						_.uSc(b, d);
						_.Rn(b.F, "API", "Project Usage Limit Update Successful");
						return d;
					} catch (d) {
						throw _.Rn(b.F, "API", "Project Usage Limit Update Failed"), d;
					} finally {
						b.A.set(false);
					}
				});
			}
		};
		_.T_.J = function(a) {
			return new (a || _.T_)();
		};
		_.T_.sa = _.Cd({
			token: _.T_,
			factory: _.T_.J,
			wa: "root"
		});
		wUc = function(a) {
			a = (a == null ? undefined : a.D7()) || "USD";
			try {
				let b;
				let c;
				return (c = (b = new Intl.NumberFormat("en", {
					style: "currency",
					currency: a,
					currencyDisplay: "symbol"
				}).formatToParts(0).find((d) => d.type === "currency")) == null ? undefined : b.value) != null ? c : a;
			} catch (b) {
				return a;
			}
		};
		_.t0 = function(a, b = true) {
			if (!a) return null;
			var c = Number(_.Ys(a, 2));
			a = a.Cl();
			c += a / 1e9;
			return b ? Math.trunc(c * 100) / 100 : c;
		};
		xUc = function(a, b = "USD") {
			if (a == null) return null;
			var c = Math.trunc(a);
			a = Math.round((a - c) * 1e9);
			if (Math.abs(a) === 1e9) {
				c += a / 1e9, a = 0;
			}
			var d = new _.R_();
			b = _.Uc(d, 1, b);
			c = _.jt(b, 2, c);
			return _.gt(c, 3, a);
		};
		yUc = function(a) {
			if (a & 1) {
				_.Kh(0, 3, 1), _.I(1, "mat-spinner", 16), _.Lh();
			}
		};
		zUc = function(a) {
			if (a & 1) {
				_.Kh(0, 5, 1), _.I(1, "mat-spinner", 16), _.Lh();
			}
		};
		_.AUc = function(a, b) {
			a = _.On(_.lk(_.BG(a), "/billing/linkedaccount"), "project", b);
			_.rd(window, a.toString());
		};
		BUc = function(a, b) {
			return _.x(function* () {
				a.A.set(true);
				var c = _.rSc(new _.oy(), b);
				c = _.In(c, 2);
				c = _.sSc(c);
				try {
					let d = yield _.tSc(a.H, c);
					_.uSc(a, d);
					_.Rn(a.F, "API", "Project Usage Limit Removal Successful");
					return d;
				} catch (d) {
					throw _.Rn(a.F, "API", "Project Usage Limit Removal Failed"), d;
				} finally {
					a.A.set(false);
				}
			});
		};
		_.u0 = class {
			constructor() {
				this.locale = _.m(_.Si);
				this.A = new _.sz(this.locale);
			}
			transform(a, b = "symbol") {
				if (!a) return null;
				var c = _.t0(a);
				var d = c.toFixed(2);
				a = a.D7();
				if (!a) return `${d}`;
				try {
					return this.A.transform(c, a, b);
				} catch (e) {
					return `${a} ${d}`;
				}
			}
		};
		_.u0.J = function(a) {
			return new (a || _.u0)();
		};
		_.u0.Wo = _.Xe({
			name: "formatMoney",
			type: _.u0,
			wk: true
		});
		var CUc;
		CUc = function(a) {
			var b = a.eT();
			if (typeof b === "number") {
				b = Math.max(0, Math.min(1e9, b)), a.eT.set(Number(b.toFixed(2)));
			}
		};
		DUc = function(a) {
			return _.x(function* () {
				CUc(a);
				var b = a.eT();
				if (b !== undefined) {
					var c = _.rSc(new _.oy(), a.projectName());
					b = xUc(b, a.S5());
					c = _.ln(c, _.R_, 2, b);
					try {
						a.Lj.set(true);
						let d = yield a.H.update(c);
						let e;
						a.Wa.close((e = _.S_(d)) != null ? e : null);
					} catch (d) {} finally {
						a.Lj.set(false);
					}
				}
			});
		};
		EUc = function(a) {
			return _.x(function* () {
				try {
					a.iDa.set(true);
					let b = yield BUc(a.H, a.projectName());
					let c;
					a.Wa.close((c = _.S_(b)) != null ? c : null);
				} catch (b) {} finally {
					a.iDa.set(false);
				}
			});
		};
		_.v0 = class {
			constructor() {
				this.ve = {
					Dpb: 309172,
					Kqb: 309167
				};
				this.A = _.m(_.qC);
				this.Wa = _.m(_.kC);
				this.H = _.m(_.T_);
				this.U = _.m(_.u0);
				this.InputType = _.lE;
				this.zKa = "/gemini-api/docs/billing#project-spend-caps";
				this.ZRb = "Your usage will pause when the cap is reached. May be subject to overages during 10 minute processing latency.";
				this.Lj = _.M(false);
				this.iDa = _.M(false);
				this.wOb = "Enter a valid amount to save your cap.";
				this.FNb = "Remove your current cap to allow unlimited usage.";
				this.PB = this.H.PB;
				this.projectName = _.W(() => this.A.project.getName());
				this.I = _.W(() => {
					var b;
					return (b = this.A.Pn) == null ? undefined : _.S_(b);
				});
				this.F = _.W(() => {
					var b;
					return (b = this.A.Pn) == null ? undefined : _.Z(b, _.R_, 3);
				});
				this.R = _.W(() => {
					var b;
					return this.F() ? (b = _.t0(this.F())) != null ? b : 0 : 0;
				});
				var a;
				this.eT = _.M(this.I() ? (a = _.t0(this.I())) != null ? a : undefined : undefined);
				this.eAb = _.W(() => wUc(this.F()));
				this.S5 = _.W(() => {
					var b;
					var c;
					return (c = (b = this.F()) == null ? undefined : b.D7()) != null ? c : "USD";
				});
				this.ys = _.W(() => this.A.project.getDisplayName() || this.A.project.Ya());
				this.yKa = _.W(() => {
					var b = this.eT();
					return b === undefined ? "" : this.R() > b ? new _.xd("Enter an amount over your current spend [{currentSpend}], otherwise your usage stops right away.").format({ currentSpend: this.U.transform(this.F()) }) : "";
				});
				this.o3a = _.W(() => this.I() === undefined || this.PB());
				this.MZ = _.W(() => this.PB() || this.eT() === undefined);
			}
			FFa() {
				this.Wa.close();
			}
			wh() {
				CUc(this);
			}
		};
		_.v0.J = function(a) {
			return new (a || _.v0)();
		};
		_.v0.ka = _.u({
			type: _.v0,
			da: [["ms-project-usage-limit-edit-dialog"]],
			features: [_.yi([_.u0])],
			ha: 24,
			ia: 21,
			la: () => [
				" Set Gemini API spend cap for �0�",
				" Learn more ",
				"Monthly spend cap",
				"�*17:1��#1:1��/#1:1��/*17:1� Remove spend cap ",
				" Cancel ",
				"�*23:1��#1:1��/#1:1��/*23:1� Save ",
				[
					"mat-dialog-title",
					"",
					1,
					"shared-dialog-header"
				],
				[1, "spend-cap"],
				[1, "description"],
				[
					"data-test-id",
					"learn-more-link",
					3,
					"documentation-path"
				],
				[1, "monthly-spend-cap-label"],
				[1, "spend-cap-input"],
				[
					"label",
					"Monthly spend cap",
					"aria-label",
					"Monthly spend cap",
					"id",
					"monthly-spend-cap-input",
					"hideLabel",
					"true",
					"rightAlignValue",
					"true",
					"cdkFocusInitial",
					"",
					3,
					"valueChange",
					"blur",
					"type",
					"value",
					"errorMessage"
				],
				[
					"external-prefix",
					"",
					1,
					"spend-cap-currency"
				],
				[1, "action-buttons"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					"data-test-id",
					"remove-spend-cap-button",
					1,
					"remove-spend-cap-button",
					3,
					"click",
					"disabled",
					"matTooltipDisabled",
					"matTooltip",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"diameter",
					"16",
					1,
					"progress-indicator"
				],
				[1, "button-right"],
				[
					"ms-button",
					"",
					"variant",
					"borderless",
					3,
					"click"
				],
				[
					"ms-button",
					"",
					"variant",
					"primary",
					"data-test-id",
					"save-spend-cap-button",
					3,
					"click",
					"disabled",
					"matTooltipDisabled",
					"matTooltip",
					"ve",
					"veClick",
					"veImpression"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "h2", 6), _.Mh(1, 0), _.H(), _.F(2, "mat-dialog-content")(3, "div", 7)(4, "span", 8), _.R(5), _.F(6, "a", 9), _.Mh(7, 1), _.H()(), _.F(8, "label", 10), _.Mh(9, 2), _.H(), _.F(10, "div", 11)(11, "ms-input-field", 12), _.J("valueChange", function(c) {
						if (!(typeof c !== "number" && c !== null)) {
							b.eT.set(c != null ? c : undefined);
						}
					})("blur", function() {
						return b.wh();
					}), _.F(12, "span", 13), _.R(13), _.H()()()(), _.F(14, "div", 14)(15, "button", 15), _.J("click", function() {
						return EUc(b);
					}), _.Kh(16, 3), _.B(17, yUc, 2, 0, "mat-spinner", 16), _.Lh(), _.H(), _.F(18, "div", 17)(19, "button", 18), _.J("click", function() {
						return b.FFa();
					}), _.Mh(20, 4), _.H(), _.F(21, "button", 19), _.J("click", function() {
						return DUc(b);
					}), _.Kh(22, 5), _.B(23, zUc, 2, 0, "mat-spinner", 16), _.Lh(), _.H()()()();
				}
				if (a & 2) {
					_.y(), _.Qh(b.ys()), _.Rh(1), _.y(4), _.S(" ", b.ZRb, " "), _.y(), _.E("documentation-path", b.zKa), _.y(5), _.E("type", b.InputType.Bnb)("value", b.eT())("errorMessage", b.yKa()), _.y(2), _.S(" ", b.eAb()), _.y(2), _.E("disabled", b.o3a())("matTooltipDisabled", !b.o3a())("matTooltip", b.FNb)("ve", b.ve.Dpb)("veClick", true)("veImpression", true), _.y(2), _.C(b.iDa() ? 17 : -1), _.y(4), _.E("disabled", b.MZ())("matTooltipDisabled", !b.MZ())("matTooltip", b.wOb)("ve", b.ve.Kqb)("veClick", true)("veImpression", true), _.y(2), _.C(b.Lj() ? 23 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.LC,
				_.mE,
				_.xC,
				_.uC,
				_.vC,
				_.zC,
				_.yC,
				_.IC,
				_.HC,
				_.Cz,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text-var);width:422px;--ms-input-width:104px}.mat-dialog-content[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:12px}.action-buttons[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;margin-left:-12px}.button-right[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:end;-webkit-justify-content:flex-end;-moz-box-pack:end;-ms-flex-pack:end;justify-content:flex-end;gap:12px}.spend-cap[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:12px;padding:0 0 24px;max-width:450px}.monthly-spend-cap-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text)}.spend-cap-input[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;color:var(--color-v3-text);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row;gap:8px;-webkit-box-align:baseline;-webkit-align-items:baseline;-moz-box-align:baseline;-ms-flex-align:baseline;align-items:baseline}.progress-indicator[_ngcontent-%COMP%]{margin-right:4px}.inline-banner[_ngcontent-%COMP%]{position:relative;border-radius:8px;border:1px solid var(--color-v3-outline-var);background:var(--color-v3-surface-container-high);color:var(--color-v3-text);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;padding:8px;overflow:auto}.inline-banner.info-inline-banner[_ngcontent-%COMP%]   .icon[_ngcontent-%COMP%]{color:var(--color-v3-text-link)}.inline-banner.error-inline-banner[_ngcontent-%COMP%]   .icon[_ngcontent-%COMP%]{color:var(--color-v3-accent-3)}.icon[_ngcontent-%COMP%]{margin-right:8px}.message[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var);overflow:auto;white-space:pre-line;word-break:break-word}"]
		});
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
		var vjd;
		tjd = function(a, b) {
			var c = new Set(a.Ap());
			return b.filter((d) => !c.has(d));
		};
		_.wjd = function(a, b) {
			a = tjd(a, b).map((c) => ujd.get(c)).filter((c) => c !== undefined);
			if (a.length === 0) return null;
			a = a.join(", ");
			return {
				errorMessage: new _.xd("Missing permissions: {MISSING_PERMISSIONS}. Please contact your project administrator for assistance.").format({ MISSING_PERMISSIONS: a }),
				actions: [{
					link: vjd,
					text: "Learn more",
					Sq: true
				}]
			};
		};
		_.Ajd = function(a) {
			return _.mj(a, _.xjd, 1, _.oj()).reduce((b, c) => {
				c = _.Z(c, _.yjd, 2);
				var d;
				c = (d = (c == null ? undefined : _.Ys(c, _.Ls(c, _.zjd, 1))) || (c == null ? undefined : _.zo(c, 2, _.zjd))) != null ? d : 0;
				return b + Number(c);
			}, 0);
		};
		_.Bjd = function(a, b) {
			return _.Uc(a, 8, b);
		};
		_.Cjd = function(a, b) {
			return _.an(a, 10, b);
		};
		_.Djd = function(a, b) {
			return _.an(a, 12, b);
		};
		_.yjd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		_.zjd = [1, 2];
		_.xjd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		Ejd = class extends _.h {
			constructor(a) {
				super(a);
			}
			getMethod() {
				return _.l(this, 4);
			}
		};
		_.j3 = function(a) {
			return _.mj(a, Ejd, 1, _.oj());
		};
		_.Fjd = function(a, b) {
			return _.$q(a.A, a.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/FetchMetricTimeSeries", b, {}, _.U4a);
		};
		vjd = (0, _.Kj)`https://docs.cloud.google.com/iam/docs/roles-permissions`;
		ujd = new Map([[14, "cloudquotas.quotas.get"], [13, "monitoring.timeSeries.list"]]);
		Gjd = [13];
		_.Hjd = function(a, b, c, d) {
			var e = "";
			var f = [];
			var g = undefined;
			if (b = _.wjd(b, Gjd)) {
				e = b.errorMessage, f = b.actions, g = 299870;
			} else {
				c.includes(0) ? (e = "Project quota tier unavailable. Please contact your project administrator for assistance.", g = 299909) : c.includes(1) && (e = "Project quota tier unknown. Please check back later.", g = 313565);
			}
			if (e) throw d && _.om(a.F, {
				content: e,
				Ne: "error",
				actions: f,
				S1: g
			}), Error(e);
		};
		_.Ijd = function(a, b, c, d) {
			if (_.pn(b) && d) throw _.om(a.F, {
				content: "Permission denied. Please contact your project administrator for assistance.",
				Ne: "error",
				S1: 299870
			}), Error("Permission denied. Please contact your project administrator for assistance.");
			if (b instanceof _.nn && b.code === 13) {
				_.Mw(a.reportError, Error("mi`" + c.getDisplayName(), { cause: b }));
			}
		};
		_.Jjd = function(a, b, c, d, e, f, g, k = []) {
			return _.x(function* () {
				if (b.length === 0) throw Error("ki");
				if (f == null || !f.length) throw Error("li");
				_.Hjd(a, d, f, true);
				try {
					var p = new _.Tx();
					var r = _.an(p, 11, b).Soa(c);
					var v = _.ot(r, 7, e);
					var w = _.Bjd(v, d.getName());
					var D = _.ot(w, 9, g);
					let G = _.Djd(_.Cjd(D, f), k);
					return yield _.Fjd(a.A, G);
				} catch (G) {
					throw _.Ijd(a, G, d, true), G;
				}
			});
		};
		_.k3 = class {
			constructor() {
				this.A = _.m(_.Zq);
				this.reportError = _.m(_.Nw);
				this.F = _.m(_.gC);
			}
		};
		_.k3.J = function(a) {
			return new (a || _.k3)();
		};
		_.k3.sa = _.Cd({
			token: _.k3,
			factory: _.k3.J,
			wa: "root"
		});
		_.Yjd = function(a) {
			var b = a.replaceAll("-", " ").replace(/\b\w/g, (c) => c.toUpperCase()).replace(/\b(tts|er)\b/gi, (c) => c.toUpperCase()).replace(/(\d+)\.0/g, "$1").replace(/\b(\d\w*[bm])\b/gi, (c) => c.toUpperCase());
			return a === "gemini-3.1-flash-image" ? `Nano Banana 2 (${b})` : a === "gemini-3-pro-image" ? `Nano Banana Pro (${b})` : a === "gemini-2.5-flash-preview-image" ? `Nano Banana (${b})` : b;
		};
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
		_.jKd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		kKd = function(a) {
			var b = new _.i$a();
			return _.$q(a.A, a.F + "/$rpc/google.internal.alkali.applications.makersuite.v1.MakerSuiteService/ListQuotaModels", b, {}, _.k$a);
		};
		_.lKd = {
			TOa: 0,
			SOa: 1,
			lsa: 20,
			wZb: 30,
			xZb: 40,
			ROa: 50
		};
		_.G5 = class {
			constructor() {
				this.F = _.m(_.Zq);
				this.A = null;
			}
			list() {
				var a = this;
				return _.x(function* () {
					if (!a.A) {
						a.A = kKd(a.F).catch((b) => {
							a.A = null;
							throw b;
						});
					}
					return a.A;
				});
			}
		};
		_.G5.J = function(a) {
			return new (a || _.G5)();
		};
		_.G5.sa = _.Cd({
			token: _.G5,
			factory: _.G5.J,
			wa: "root"
		});
		_.mKd = function(a) {
			return new Date(a.getFullYear(), a.getMonth(), 1, 0, 0, 0);
		};
		_.nKd = function(a) {
			switch (a) {
				case 1: return 1;
				case 2: return 7;
				case 3: return 28;
				case 4: return 90;
				case 6: return a = new Date(), new Date(a.getFullYear(), a.getMonth() + 1, 0, 23, 59, 59).getDate() - _.mKd(a).getDate() + 1;
				default: return 0;
			}
		};
		_.oKd = function({ w2a: a = false } = {}) {
			return a ? _.DG.filter((b) => _.nKd(b.value) > 1) : _.DG;
		};
		pKd = function(a) {
			if (a & 1) {
				let b = _.n();
				_.F(0, "label", 6);
				_.J("click", function() {
					_.q(b);
					_.K();
					var c = _.O(3);
					return _.t(c.open());
				});
				_.Mh(1, 1);
				_.H();
			}
		};
		qKd = function(a, b) {
			if (a & 1) {
				_.F(0, "mat-option", 5), _.R(1), _.H();
			}
			if (a & 2) {
				a = b.V, _.E("value", a.value)("ve", a.ve)("veImpression", true)("veClick", true), _.y(), _.U(a.label);
			}
		};
		rKd = function(a, b) {
			if (a & 1) {
				_.I(0, "ms-quota-tier-badge", 4);
			}
			if (a & 2) {
				_.E("quotaTier", b);
			}
		};
		sKd = function(a) {
			if (a & 1) {
				_.F(0, "span", 5), _.Mh(1, 0), _.H();
			}
		};
		tKd = function(a) {
			if (a & 1) {
				_.Mh(0, 1, 1);
			}
			if (a & 2) {
				a = _.K(), _.Qh(a.text), _.Rh(0);
			}
		};
		uKd = function(a, b) {
			if (a & 1) {
				let c = _.n();
				_.F(0, "button", 9);
				_.J("click", function() {
					var d = _.q(c);
					return _.t(d.click());
				});
				_.Kh(1, 1);
				_.B(2, tKd, 1, 1);
				_.Lh();
				_.H();
			}
			if (a & 2) {
				a = _.K(2);
				let c;
				_.E("variant", a.i$a() ? "icon-borderless" : "borderless")("iconName", a.S.DESCRIPTION)("matTooltip", (c = b.tooltip) != null ? c : "")("ve", b.ve)("veClick", true);
				_.wh("data-test-id", b.t1);
				_.y(2);
				_.C(a.i$a() ? -1 : 2);
			}
		};
		vKd = function(a, b) {
			if (a & 1) {
				let c = _.n();
				_.F(0, "button", 10);
				_.J("click", function() {
					var d = _.q(c);
					return _.t(d.click());
				});
				_.R(1);
				_.H();
			}
			if (a & 2) {
				let c;
				_.E("iconName", b.icon)("matTooltip", (c = b.tooltip) != null ? c : "")("ve", b.ve)("veClick", true)("disabled", b.disabled);
				_.wh("data-test-id", b.t1);
				_.y();
				_.S(" ", b.text, " ");
			}
		};
		wKd = function(a) {
			if (a & 1) {
				_.F(0, "div", 6), _.B(1, uKd, 3, 7, "button", 7), _.B(2, vKd, 2, 7, "button", 8), _.H();
			}
			if (a & 2) {
				let b;
				let c;
				a = _.K();
				_.y();
				_.C((b = a.TE()) ? 1 : -1, b);
				_.y();
				_.C((c = a.Gna()) ? 2 : -1, c);
			}
		};
		_.H5 = class {
			constructor() {
				this.ve = _.V(0);
				this.Cs = _.V();
				this.z1 = _.Li(_.oKd());
				this.ena = _.Ki();
				this.du = _.V(true);
				this.HBb = _.W(() => {
					var a = this.Cs();
					var b = this.z1();
					var c;
					var d;
					return (c = (d = b.find((e) => e.value === a)) != null ? d : b[0]) == null ? undefined : c.value;
				});
			}
		};
		_.H5.J = function(a) {
			return new (a || _.H5)();
		};
		_.H5.ka = _.u({
			type: _.H5,
			da: [["ms-timerange-selector"]],
			inputs: {
				ve: [1, "ve"],
				Cs: [1, "selectedTimeRange"],
				z1: [1, "timeRangeOptions"],
				du: [1, "showSelectorLabel"]
			},
			outputs: { ena: "onTimeRangeChange" },
			ha: 6,
			ia: 5,
			la: () => [
				["timeRangeSelect", ""],
				"Time Range",
				[
					"id",
					"time-range-select-label",
					"for",
					"time-range-select-input"
				],
				[
					"appearance",
					"outline",
					"subscriptSizing",
					"dynamic",
					1,
					"time-range-selector-form-field"
				],
				[
					"id",
					"time-range-select-input",
					"aria-labelledby",
					"time-range-select-label",
					3,
					"selectionChange",
					"value",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					3,
					"value",
					"ve",
					"veImpression",
					"veClick"
				],
				[
					"id",
					"time-range-select-label",
					"for",
					"time-range-select-input",
					3,
					"click"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, pKd, 2, 0, "label", 2), _.F(1, "mat-form-field", 3)(2, "mat-select", 4, 0), _.J("selectionChange", function(c) {
						return b.ena.emit(c.value);
					}), _.Ah(4, qKd, 2, 5, "mat-option", 5, _.zh), _.H()();
				}
				if (a & 2) {
					_.C(b.du() ? 0 : -1), _.y(2), _.E("value", b.HBb())("ve", b.ve())("veImpression", true)("veClick", true), _.y(2), _.Bh(b.z1());
				}
			},
			dependencies: [
				_.JD,
				_.$D,
				_.ZD,
				_.dE,
				_.bE,
				_.QB,
				_.Cz,
				_.Bz
			],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px}[_nghost-%COMP%] > label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0;color:var(--color-v3-text-var)}.time-range-selector-form-field[_ngcontent-%COMP%]{width:100%}"]
		});
		_.I5 = class {
			constructor() {
				this.Ht = _.V();
				this.TE = _.V();
				this.Gna = _.V();
				this.D$a = _.V(false);
				this.P$a = _.V(true);
				this.A = _.m(_.EG);
				this.F = _.m(_.ZC);
				this.i$a = this.F.A.Il;
				this.xa = this.A.A;
				this.kAb = _.W(() => {
					var a;
					return this.P$a() ? (a = this.xa()) == null ? undefined : a.ah() : undefined;
				});
				this.isInternal = _.W(() => {
					var a;
					return (a = this.xa()) == null ? undefined : _.Pm(a, 16);
				});
				this.dFb = _.W(() => !!this.TE() || !!this.Gna());
				this.oFb = _.W(() => !!this.TE() && !this.Gna());
				this.S = _.Dk;
			}
		};
		_.I5.J = function(a) {
			return new (a || _.I5)();
		};
		_.I5.ka = _.u({
			type: _.I5,
			da: [["ms-dashboard-header"]],
			inputs: {
				Ht: [1, "headerText"],
				TE: [1, "learnMoreButton"],
				Gna: [1, "primaryButton"],
				D$a: [1, "showInternalLabel"],
				P$a: [1, "showQuotaTierBadge"]
			},
			ha: 7,
			ia: 6,
			la: () => [
				"Internal",
				"�*2:1� �0:1� �/*2:1�",
				[1, "header-container"],
				[1, "title-container"],
				[3, "quotaTier"],
				[
					"matTooltipPosition",
					"below",
					"matTooltip",
					"First-party project has higher quota allocations",
					1,
					"first-party-label"
				],
				[1, "cta-buttons"],
				[
					"ms-button",
					"",
					"matTooltipPosition",
					"below",
					"aria-label",
					"Learn more",
					3,
					"variant",
					"iconName",
					"matTooltip",
					"ve",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"primary",
					"matTooltipPosition",
					"below",
					3,
					"iconName",
					"matTooltip",
					"ve",
					"veClick",
					"disabled"
				],
				[
					"ms-button",
					"",
					"matTooltipPosition",
					"below",
					"aria-label",
					"Learn more",
					3,
					"click",
					"variant",
					"iconName",
					"matTooltip",
					"ve",
					"veClick"
				],
				[
					"ms-button",
					"",
					"variant",
					"primary",
					"matTooltipPosition",
					"below",
					3,
					"click",
					"iconName",
					"matTooltip",
					"ve",
					"veClick",
					"disabled"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 2)(1, "div", 3)(2, "h2"), _.R(3), _.H(), _.B(4, rKd, 1, 1, "ms-quota-tier-badge", 4), _.B(5, sKd, 2, 0, "span", 5), _.H(), _.B(6, wKd, 3, 2, "div", 6), _.H();
				}
				if (a & 2) {
					let c;
					_.P("learn-more-only", b.oFb());
					_.y(3);
					_.U(b.Ht());
					_.y();
					_.C((c = b.kAb()) ? 4 : -1, c);
					_.y();
					_.C(b.D$a() && b.isInternal() ? 5 : -1);
					_.y();
					_.C(b.dFb() ? 6 : -1);
				}
			},
			dependencies: [
				_.Yy,
				_.fE,
				_.Cz,
				_.Bz,
				_.IC,
				_.HC
			],
			styles: [".title-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:8px}.title-container[_ngcontent-%COMP%]   h2[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:24px;font-weight:600;line-height:32px}@media screen and (max-width:768px){.title-container[_ngcontent-%COMP%]{-webkit-box-ordinal-group:1;-webkit-order:0;-moz-box-ordinal-group:1;-ms-flex-order:0;order:0;-webkit-align-self:flex-start;-ms-flex-item-align:start;align-self:flex-start;width:100%}}.header-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;margin-bottom:20px;gap:12px}.title-container[_ngcontent-%COMP%]{-webkit-flex-wrap:wrap;-ms-flex-wrap:wrap;flex-wrap:wrap}.first-party-label[_ngcontent-%COMP%]{border-radius:8px;padding:1px 6px 1px 5px;border:1px solid var(--color-v3-outline);background-color:var(--color-v3-surface-container-high);color:var(--color-v3-text);display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:5px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-accent-1);border-color:var(--color-v3-accent-1)}.first-party-label[_ngcontent-%COMP%]:before{content:\"\";width:6px;aspect-ratio:1/1;border-radius:50%;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.first-party-label.enabled[_ngcontent-%COMP%]:before, .first-party-label.green[_ngcontent-%COMP%]:before, .first-party-label.new[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-4)}.first-party-label.gray[_ngcontent-%COMP%]:before, .first-party-label.not-enabled[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-var)}.first-party-label.confidential[_ngcontent-%COMP%]:before, .first-party-label.orange[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-1)}.first-party-label.blue[_ngcontent-%COMP%]:before, .first-party-label.paid[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-link)}.first-party-label.alert[_ngcontent-%COMP%]:before, .first-party-label.red[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-3)}.first-party-label.hide-circle[_ngcontent-%COMP%]:before{display:none}.first-party-label[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-1)}.cta-buttons[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:8px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center}.subheader[_ngcontent-%COMP%]{margin-top:8px;width:auto;font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:16px;font-weight:600;line-height:24px}@media screen and (max-width:600px){.header-container[_ngcontent-%COMP%]{-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start}.header-container.learn-more-only[_ngcontent-%COMP%]{-webkit-box-orient:horizontal;-webkit-box-direction:normal;-webkit-flex-direction:row;-moz-box-orient:horizontal;-moz-box-direction:normal;-ms-flex-direction:row;flex-direction:row}.subheader[_ngcontent-%COMP%]{width:100%}}"]
		});
		var yKd = function(a) {
			return _.wo(new _.qo(), _.oo(_.oo(_.oo(new _.no(), "year", _.so(new _.qo(), a.getFullYear())), "month", _.so(new _.qo(), a.getMonth() + 1)), "day", _.so(new _.qo(), a.getDate())));
		};
		var zKd = function(a, b) {
			if (b) {
				var c = new _.no();
				_.oo(c, "goog-generativelanguage-model", _.to(new _.qo(), b.replace(/[-.]/g, "").toLowerCase()));
				_.oo(a, "billingMetadataFilter", _.wo(new _.qo(), _.oo(new _.no(), "labels", _.wo(new _.qo(), c))));
			}
		};
		var AKd = function(a) {
			return a.map((b) => ({
				value: _.l(b, 1),
				label: _.Yjd(_.l(b, 1)),
				data: b
			}));
		};
		var CKd = function(a) {
			var b = _.oo(new _.no(), "customer", _.to(new _.qo(), "aistudio"));
			if (a.billingAccountId) {
				_.oo(b, "billingAccountId", _.to(new _.qo(), a.billingAccountId));
			}
			if (a.projectId) {
				_.oo(b, "projectIds", _.vo(new _.qo(), _.oBa(new _.uo(), _.to(new _.qo(), a.projectId))));
			}
			_.oo(b, "serviceIds", _.vo(new _.qo(), _.oBa(new _.uo(), _.to(new _.qo(), "AEFD-7695-64FA"))));
			var c;
			var d = (c = a.timeRange) != null ? c : 6;
			if (d === 6) _.oo(b, "startDate", yKd(_.mKd(new Date())));
			else {
				c = new Date();
				let f = new Date(c);
				f.setDate(c.getDate() - (_.nKd(d) - 1));
				_.oo(b, "startDate", yKd(f));
				_.oo(b, "endDate", yKd(c));
			}
			var e;
			d = ["BillingCostOverview_EnableCharting:true", ...(e = a.Gia) != null ? e : []];
			_.oo(b, "featureFlags", _.vo(new _.qo(), BKd(new _.uo(), d.map((f) => _.to(new _.qo(), f)))));
			if (a.kTa) {
				zKd(b, a.kTa);
			}
			return b;
		};
		var DKd = function(a) {
			return _.x(function* () {
				var b = a.JY();
				switch (b) {
					case 1012: return yield (0, _.Sp)("VmGkac"), _.wQ;
					case 1007: break;
					case 1e3: return {Lqb: b} = yield J5(), b;
					case 1001: return yield (0, _.Sp)("qrFA7b"), _.MZ;
					case 1022: return {Mqb: b} = yield J5(), b;
					case 1013: break;
					case 1004: return {Nqb: b} = yield J5(), b;
					case 1005:
						b = a.Al();
						if (_.sn(b, _.kYa, 3)) return {Tqb: b} = yield J5(), b;
						({Oqb: b} = yield J5());
						return b;
					case 1021: return {Pqb: b} = yield J5(), b;
					case 1003: return {Qqb: b} = yield J5(), b;
					case 1026: return {Rqb: b} = yield J5(), b;
					case 1018: break;
					case 1023: return {Sqb: b} = yield J5(), b;
					case 1016: return {Uqb: b} = yield J5(), b;
					case 1017: break;
					case 1015: break;
					case 1009: return {Vqb: b} = yield J5(), b;
					case 1010:
					case 1011:
					case 1014: break;
					case 1002: return {Wqb: b} = yield J5(), b;
					case 1025: return {Xqb: b} = yield J5(), b;
					case 1006: return {Yqb: b} = yield J5(), b;
					case 0: break;
					default: _.sb(b, "Unknown component type");
				}
			});
		};
		var LKd = function(a) {
			return a !== undefined && a !== null;
		};
		var OKd = function(a) {
			switch (a) {
				case "pdf": return "application/pdf";
				case "bin":
				case "p12": return "application/octet-stream";
				case "jpg": return "image/jpeg";
				case "png": return "image/png";
				case "json": return "attachment/json";
				case "csv": return "attachment/csv";
				default: return "attachment/text";
			}
		};
		var PKd = function(a) {
			return _.Gk(a.getSeconds()) * 1e3 + a.Cl() / 1e6;
		};
		var $Kd = function({ value: a, type: b }) {
			switch (b) {
				case "null": return _.PX(new _.QX(), QKd());
				case "bool": return _.PX(new _.QX(), _.XCc(new _.RX(), a));
				case "int64": return _.PX(new _.QX(), _.$Cc(new _.RX(), _.nb(a)));
				case "uint64": return _.PX(new _.QX(), RKd(new _.RX(), _.nb(a)));
				case "double": return _.PX(new _.QX(), SKd(new _.RX(), a));
				case "string": return _.PX(new _.QX(), TKd(new _.RX(), a));
				case "bytes": return _.PX(new _.QX(), UKd(new _.RX(), a));
				case "duration": return VKd(new _.QX(), WKd(XKd("duration"), _.PX(new _.QX(), TKd(new _.RX(), a.toString()))));
				case "timestamp": return VKd(new _.QX(), WKd(XKd("timestamp"), _.PX(new _.QX(), TKd(new _.RX(), a.toString()))));
				case "list": return YKd(new _.QX(), ZKd(new _.lEc(), a.elements.map($Kd)));
				case "map": return aLd(new _.QX(), bLd(new _.kEc(), Array.from(a.entries(), ([c, d]) => {
					var e = new _.jEc();
					c = $Kd(c);
					return _.Ap(e, 3, _.nEc, c).setValue($Kd(d));
				})));
				case "type": return _.kLc(cLd(new _.lY(), a));
				case "error": throw Error("Di");
				default: return _.sb(b, undefined);
			}
		};
		var eLd = function(a) {
			a = a.reduce((b, c) => {
				var d;
				return b.set(c.Fr, ((d = b.get(c.Fr)) != null ? d : 0) + 1);
			}, new Map());
			Array.from(a.entries()).filter(([, b]) => b > 1).sort((b, c) => b[0] - c[0]);
		};
		var BKd = function(a, b) {
			return _.Zm(a, 1, b);
		};
		var QKd = function() {
			var a = new _.RX();
			return _.pt(a, 1, _.fY, 0);
		};
		var RKd = function(a, b) {
			return _.Ps(a, 4, _.fY, b == null ? b : _.Eba(b));
		};
		var SKd = function(a, b) {
			return _.Ps(a, 5, _.fY, b == null ? b : _.tb(b));
		};
		var TKd = function(a, b) {
			return _.mt(a, 6, _.fY, b);
		};
		var UKd = function(a, b) {
			return _.Ps(a, 7, _.fY, _.eb(b, false, true));
		};
		var cLd = function(a, b) {
			return _.Uc(a, 1, b);
		};
		var bLd = function(a, b) {
			return _.Zm(a, 2, b);
		};
		var ZKd = function(a, b) {
			return _.Zm(a, 1, b);
		};
		var XKd = function(a) {
			var b = new _.mEc();
			return _.Uc(b, 2, a);
		};
		var WKd = function(a, b) {
			return _.Us(a, 3, _.QX, b);
		};
		var VKd = function(a, b) {
			return _.Ap(a, 6, _.mY, b);
		};
		var YKd = function(a, b) {
			return _.Ap(a, 7, _.mY, b);
		};
		var aLd = function(a, b) {
			return _.Ap(a, 8, _.mY, b);
		};
		var nLd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var oLd = function(a) {
			return _.mj(a, nLd, 1, _.oj());
		};
		var pLd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var qLd = class extends _.h {
			constructor(a) {
				super(a);
			}
			Sb() {
				return _.l(this, 1);
			}
			Fe() {
				return _.zn(this, 1);
			}
			getFileName() {
				return _.l(this, 2);
			}
		};
		var rLd = class extends _.h {
			constructor(a) {
				super(a);
			}
			getUrl() {
				return _.l(this, 1);
			}
			getFileName() {
				return _.l(this, 2);
			}
		};
		var sLd = class extends _.h {
			constructor(a) {
				super(a);
			}
			getId() {
				return _.l(this, 1);
			}
			qm() {
				return _.zn(this, 1);
			}
		};
		var tLd = function(a) {
			return _.Z(a, _.kp, 1);
		};
		var uLd = function(a) {
			return _.sn(a, _.kp, 1);
		};
		var vLd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var zLd = function(a) {
			a.signals.clear();
			for (let b of a.F.entries()) a.signals.set(b[0], b[1]);
		};
		var BLd = function(a, b) {
			return (a = a.forms.get(b)) ? Array.from(a.components.values()) : [];
		};
		var K5 = class {};
		K5.J = function(a) {
			return new (a || K5)();
		};
		K5.qc = _.Ve({ type: K5 });
		K5.oc = _.Dd({
			vd: [{
				Da: _.rC,
				Mf: _.PZ
			}],
			imports: [_.xC]
		});
		var L5 = class extends _.PZ {
			constructor() {
				super();
				this.X.push("cm-dialog");
			}
		};
		L5.J = function(a) {
			return new (a || L5)();
		};
		L5.sa = _.Cd({
			token: L5,
			factory: L5.J,
			wa: "root"
		});
		var M5 = class {};
		M5.J = function(a) {
			return new (a || M5)();
		};
		M5.qc = _.Ve({ type: M5 });
		M5.oc = _.Dd({
			vd: [{
				Da: _.rC,
				Mf: L5
			}],
			imports: [K5, K5]
		});
		var DLd = new _.he("AiStudioClientConfig");
		_.ELd = {
			Da: DLd,
			ke: () => {
				var a = _.m(_.Qu);
				return {
					endpoint: a.Ta,
					szb: a.F,
					Ro: String(a.A)
				};
			}
		};
		var FLd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var N5 = class {
			get A() {
				return (N5.A++).toString();
			}
		};
		N5.A = 10;
		N5.J = function(a) {
			return new (a || N5)();
		};
		N5.sa = _.Cd({
			token: N5,
			factory: N5.J,
			wa: "root"
		});
		var GLd = class {
			constructor(a) {
				this.A = a;
			}
		};
		var HLd = class extends _.bJc {
			constructor() {
				super("colorScheme", "0-0");
			}
		};
		var O5 = class extends HLd {
			constructor() {
				super(...arguments);
				this.service = _.m(_.HG);
				this.Pa = _.m(_.Xf);
			}
			F() {
				return _.YCc(_.Qd(this.service.Hf));
			}
			A() {
				return _.Bk(this.service.Hf, { Pa: this.Pa }).pipe(_.uf((a) => _.YCc(a)));
			}
		};
		O5.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(O5)))(b || O5);
			};
		})();
		O5.sa = _.Cd({
			token: O5,
			factory: O5.J,
			wa: "root"
		});
		var P5 = class extends _.dJc {
			constructor() {
				super({ colorScheme: _.m(O5) });
			}
		};
		P5.J = function(a) {
			return new (a || P5)();
		};
		P5.sa = _.Cd({
			token: P5,
			factory: P5.J,
			wa: "root"
		});
		var ILd = class extends _.h {
			constructor(a) {
				super(a);
			}
			m0a() {
				return _.l(this, 2);
			}
			AJa(a) {
				return _.Lj(this, 2, a);
			}
		};
		var JLd = class extends _.h {
			constructor(a) {
				super(a);
			}
			Bja() {
				return _.l(this, 3);
			}
			getP2Metadata() {
				return _.Z(this, _.qJ, 17);
			}
			setP2Metadata(a) {
				return _.ln(this, _.qJ, 17, a);
			}
			getPlatformMetadata() {
				return _.Z(this, _.Byb, 16);
			}
			Ya() {
				return _.l(this, 19);
			}
		};
		var KLd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var LLd = class extends _.h {
			constructor(a) {
				super(a);
			}
		};
		var MLd = class extends _.h {
			constructor(a) {
				super(a);
			}
			A() {
				return _.Z(this, LLd, 1);
			}
			Mx(a) {
				return _.ln(this, LLd, 1, a);
			}
			Tm() {
				return _.In(this, 1);
			}
		};
		var NLd = new _.iw("/google.internal.cloud.clientapi.sdui.BillingSduiService/GetSdui", KLd, MLd, (a) => a.serialize(), _.bd(MLd));
		var PLd = function(a, b) {
			var c = {};
			return new _.ef((d) => {
				var e = a.A.serverStreaming(a.F + "/$rpc/google.internal.cloud.clientapi.sdui.BillingSduiService/GetSdui", b, c || {}, NLd);
				e.on("data", (f) => {
					d.next(f);
				});
				e.on("error", (f) => {
					d.error(f);
				});
				e.on("status", (f) => {
					if (f.code !== 0) {
						d.error(new _.nn(f.code, f.details, f.metadata));
					}
				});
				e.on("end", () => {
					d.complete();
				});
				return () => {
					e.cancel();
				};
			});
		};
		var QLd;
		QLd = _.Zn(_.no);
		SLd = function(a, b) {
			b = RLd(a, b);
			return PLd(a.F, b).pipe(_.uf((c) => c.A()));
		};
		RLd = function(a, b) {
			var c = new JLd();
			var d = new ILd();
			d = _.Lj(d, 1, "en");
			c = _.ln(c, ILd, 22, d);
			d = new KLd();
			d = _.Uc(d, 4, a.H.A);
			d = _.Uc(d, 1, b.intent);
			c = _.ln(d, JLd, 3, c);
			a = _.cJc(a.A);
			a = _.QPa(c, 6, a, _.sY);
			c = new FLd();
			c = _.cn(c, 1, 4);
			c = _.Lj(c, 2, "8");
			a = _.ln(a, FLd, 8, c);
			if (QLd(b.payload)) {
				_.ln(a, _.no, 2, b.payload);
			} else {
				b = _.X5b(b.payload), _.ln(a, _.no, 2, b);
			}
			return a;
		};
		_.Q5 = class {
			constructor() {
				this.H = _.m(N5);
				this.F = _.m(OLd);
				this.A = _.m(P5);
			}
		};
		_.Q5.J = function(a) {
			return new (a || _.Q5)();
		};
		_.Q5.sa = _.Cd({
			token: _.Q5,
			factory: _.Q5.J
		});
		_.R5 = class {
			constructor() {
				this.A = _.m(_.Q5);
			}
		};
		_.R5.J = function(a) {
			return new (a || _.R5)();
		};
		_.R5.sa = _.Cd({
			token: _.R5,
			factory: _.R5.J
		});
		var TLd;
		var ULd;
		TLd = {
			value: "all",
			label: "All Models"
		};
		ULd = function(a) {
			_.x(function* () {
				try {
					let b = yield a.R.list();
					a.A.set(b);
				} catch (b) {
					_.Mw(a.H, Error("Ai", { cause: b }));
				}
			});
		};
		VLd = function(a, b) {
			var c;
			var d;
			var e = (c = a.er().find((g) => g !== TLd)) == null ? undefined : (d = c.data) == null ? undefined : _.l(d, 6);
			var f;
			return CKd(Object.assign({}, b, {
				kTa: e,
				Gia: [...a.X(), ...(f = b.Gia) != null ? f : []]
			}));
		};
		_.S5 = class {
			constructor() {
				this.U = _.m(_.R5);
				this.R = _.m(_.G5);
				this.H = _.m(_.Nw);
				this.I = _.m(_.Op);
				this.aa = this.I.getFlag(_.aGb);
				this.X = _.W(() => [`BillingCostOverview_AutoflexCalculation:${this.aa}`]);
				this.A = _.M(null);
				this.Ln = _.W(() => {
					var a;
					var b;
					var c = (b = (a = this.A()) == null ? undefined : _.mj(a, _.jKd, 1, _.oj())) != null ? b : [];
					return [TLd, ...AKd(c)];
				});
				this.F = _.M([TLd]);
				this.er = this.F.asReadonly();
			}
			Bba(a) {
				this.F.set(a);
			}
		};
		_.S5.J = function(a) {
			return new (a || _.S5)();
		};
		_.S5.sa = _.Cd({
			token: _.S5,
			factory: _.S5.J,
			wa: "root"
		});
		var T5 = class {};
		T5.J = function(a) {
			return new (a || T5)();
		};
		T5.sa = _.Cd({
			token: T5,
			factory: T5.J
		});
		var U5 = class extends _.TJc {
			A(a) {
				return _.x(function* () {
					var b = yield DKd(a);
					if (b) return {
						primitiveType: b,
						inputs: new Map([["comp", a]])
					};
				});
			}
		};
		U5.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(U5)))(b || U5);
			};
		})();
		U5.sa = _.Cd({
			token: U5,
			factory: U5.J
		});
		var V5 = class extends _.EZ {};
		V5.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(V5)))(b || V5);
			};
		})();
		V5.sa = _.Cd({
			token: V5,
			factory: V5.J
		});
		var W5 = class extends _.jP {
			constructor() {
				super(...arguments);
				this.F = "ai-studio-sdui-viewer-tokens-856794408";
			}
		};
		W5.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(W5)))(b || W5);
			};
		})();
		W5.sa = _.Cd({
			token: W5,
			factory: W5.J,
			wa: "root"
		});
		var WLd = new _.he("SduiActionHandler");
		var X5 = class extends Window {};
		X5.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(X5)))(b || X5);
			};
		})();
		X5.sa = _.Cd({
			token: X5,
			factory: function(a) {
				var b = null;
				if (a) {
					b = new (a || X5)();
				} else {
					b = window;
				}
				return b;
			},
			wa: "root"
		});
		var Y5 = class {
			constructor() {
				this.A = _.m(X5);
				this.Fr = 7;
			}
			Zg(a) {
				a = _.GY(a);
				if (_.zn(a, 1)) {
					try {
						_.uw(a.getUrl());
					} catch (b) {
						return;
					}
					if (a = _.jd(a.getUrl())) {
						_.yea(this.A.location, a);
					}
				}
			}
			reset() {}
		};
		Y5.J = function(a) {
			return new (a || Y5)();
		};
		Y5.sa = _.Cd({
			token: Y5,
			factory: Y5.J
		});
		new _.he("GM2_SNACK_BAR_OPTIONS", {
			wa: "root",
			factory: () => ({ yl: false })
		});
		var YLd = class {
			constructor() {
				this.config = _.m(_.mjb);
				this.xb = this.config.content;
				this.qr = _.fj(this.xb, _.BFc, 1015, _.CY);
				this.Lg = this.oj = this.uKa = _.Ni("snackbarContainerElement", Object.assign({}, {}, { read: _.Jf }));
				this.WPb = _.sn(this.qr, _.xY, 3) && _.Z(this.qr, _.xY, 3).hasLabel() && _.Z(this.qr, _.xY, 3).zo().length > 0;
				var a;
				var b;
				var c;
				var d;
				this.X3 = yLd(new _.tY(), xLd(_.PFc(new _.QFc(), new _.tY().setText(wLd(new _.uFc(), (c = (a = _.Z(this.qr, _.xY, 3)) == null ? undefined : a.Bl()) != null ? c : ""))), (d = (b = _.Z(this.qr, _.xY, 3)) == null ? undefined : b.zo()) != null ? d : []));
				var e = _.m(_.xZ);
				var f = _.m(_.Hu);
				_.Kg((g) => {
					var k = _.wZ(e, this.xb).subscribe(() => {
						f.lb();
					});
					g(() => {
						k.unsubscribe();
					});
				});
			}
		};
		YLd.J = function(a) {
			return new (a || YLd)();
		};
		YLd.ka = _.u({
			type: YLd,
			da: [["sdui-snackbar"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.uKa, XLd, 5, _.Jf);
				}
				if (a & 2) {
					_.ki();
				}
			},
			Ua: 2,
			Ja: function(a) {
				if (a & 2) {
					_.pi("display", "contents");
				}
			},
			ha: 5,
			ia: 3,
			la: [
				["snackbarContainerElement", ""],
				[
					"sdui-component",
					"",
					3,
					"comp"
				],
				["sdui-token-host", ""],
				[1, "cfc-snack-bar-content"],
				[1, "cfc-snack-bar-actions"],
				[3, "comp"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Gh(0, 1, 0), _.F(2, "div", 2), _.B(3, EKd, 2, 1, "div", 3), _.B(4, FKd, 2, 1, "div", 4), _.H(), _.Hh();
				}
				if (a & 2) {
					_.E("comp", b.xb), _.y(3), _.C(b.qr.Fe() ? 3 : -1), _.y(), _.C(b.WPb ? 4 : -1);
				}
			},
			dependencies: [
				_.tz,
				_.LZ,
				_.zZ,
				_.cR
			],
			styles: ["[sdui-token-host][_ngcontent-%COMP%]{display:contents}"]
		});
		var ZLd = class {
			constructor() {
				this.Z0 = _.m(_.cC);
				this.gzb = "Button to close the snackbar.";
			}
			close() {
				this.Z0.pj();
			}
		};
		ZLd.J = function(a) {
			return new (a || ZLd)();
		};
		ZLd.ka = _.u({
			type: ZLd,
			da: [["ng-component"]],
			features: [_.mh([_.cR])],
			ha: 7,
			ia: 1,
			la: [
				[1, "sdui-snackbar-spacing"],
				[3, "click"],
				[1, "hover-indicator"],
				[1, "focus-indicator"],
				"viewBox;0 0 18 18;fit;;preserveAspectRatio;xMidYMid meet;focusable;false".split(";"),
				[
					"d",
					"M10.5 9l4.906-4.907-1.5-1.5L9 7.503l-4.907-4.91-1.5 1.5L7.503 9l-4.907 4.907 1.5 1.5L9\n         10.497l4.907 4.907 1.5-1.5L10.497 9z",
					"fill-rule",
					"evenodd"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 0), _.I(1, "sdui-snackbar"), _.H(), _.F(2, "button", 1), _.J("click", function() {
						return b.close();
					}), _.I(3, "span", 2)(4, "span", 3), _.Ee(), _.F(5, "svg", 4), _.I(6, "path", 5), _.H()();
				}
				if (a & 2) {
					_.y(2), _.wh("aria-label", b.gzb);
				}
			},
			dependencies: [YLd],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:var(--sdui-ref-space-4)}.sdui-snackbar-spacing[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:1;-webkit-flex:1 1 0;-moz-box-flex:1;-ms-flex:1 1 0px;flex:1 1 0}.sdui-snackbar-spacing[_ngcontent-%COMP%]     .cfc-snack-bar-content{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:1;-webkit-flex:1 1 0;-moz-box-flex:1;-ms-flex:1 1 0px;flex:1 1 0}.sdui-snackbar-spacing[_ngcontent-%COMP%]     .cfc-snack-bar-actions{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:0;-webkit-flex:0 0 auto;-moz-box-flex:0;-ms-flex:0 0 auto;flex:0 0 auto;margin:0}button[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;background:none;border:none;border-radius:50%;color:var(--sdui-sys-color-inverse-on-surface);cursor:pointer;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-flex:0;-webkit-flex:0 0 auto;-moz-box-flex:0;-ms-flex:0 0 auto;flex:0 0 auto;height:40px;-webkit-box-pack:center;-webkit-justify-content:center;-moz-box-pack:center;-ms-flex-pack:center;justify-content:center;margin:0;padding:0;position:relative;width:40px}button[_ngcontent-%COMP%] > svg[_ngcontent-%COMP%]{fill:currentColor;height:18px;width:18px}button[_ngcontent-%COMP%]   .focus-indicator[_ngcontent-%COMP%], button[_ngcontent-%COMP%]   .hover-indicator[_ngcontent-%COMP%]{background-color:transparent;border-radius:50%;height:100%;left:0;pointer-events:none;position:absolute;top:0;width:100%;z-index:1}button[_ngcontent-%COMP%]:hover:not(:focus-within) > .hover-indicator[_ngcontent-%COMP%]{background-color:var(--sdui-sys-color-inverse-on-surface);opacity:.24}button[_ngcontent-%COMP%]:active > .hover-indicator[_ngcontent-%COMP%]{background-color:var(--sdui-sys-color-inverse-on-surface);opacity:.12}button[_ngcontent-%COMP%]:focus-within:not(:active) > .focus-indicator[_ngcontent-%COMP%]{outline:solid 2px var(--sdui-sys-color-primary)}"]
		});
		var Z5 = class {
			constructor() {
				this.F = _.m(_.gC);
				this.A = _.m(_.$h, { optional: true });
				this.Fr = 8;
			}
			Zg(a, b) {
				var c = _.fj(a, _.VFc, 8, _.FY);
				var d;
				a = this.F;
				var e = { content: c.Sb() };
				var f = (d = this.A) != null ? d : b;
				b = _.l(c, 2);
				d = GKd(_.Lm(c, 5));
				a: switch (c = _.Lm(c, 6)) {
					case 1:
						c = "top";
						break a;
					case 2:
					case 0:
						c = "bottom";
						break a;
					default: c = _.sb(c, undefined);
				}
				_.mm(a, ZLd, {
					data: e,
					bf: f,
					GP: b,
					o8: d,
					Cdb: c
				});
			}
			reset() {}
		};
		Z5.J = function(a) {
			return new (a || Z5)();
		};
		Z5.sa = _.Cd({
			token: Z5,
			factory: Z5.J
		});
		var $5 = class {
			constructor() {
				this.xb = _.Li.required();
				this.menu = _.W(() => {
					var c = this.xb();
					return _.fj(c, _.FFc, 1018, _.CY);
				});
				this.id = _.Li.required();
				this.triggerId = _.Li.required();
				this.Lg = this.oj = this.GEa = _.Ni("menuElement", Object.assign({}, {}, { read: _.Jf }));
				this.Aoa = _.hi();
				this.F = _.W(() => this.Aoa().map(JKd).filter((c) => !!c));
				this.A = _.bB(_.aB(_.YA(_.ZA(new _.eB(this.F, _.m(_.Xf)), true)))).Ox(() => false);
				this.nKb = (c) => {
					this.A.sk(c);
				};
				_.cj({ write: () => {
					this.Aoa().forEach((c) => {
						var d;
						var e;
						if (!((d = c.Lg()) == null || (e = d.nativeElement) == null)) {
							e.setAttribute("role", "menuitem");
						}
					});
				} });
				_.cj({ write: () => {
					if (this.Aoa().length !== 0) {
						_.Qd(() => {
							_.dB(this.A);
						});
					}
				} });
				var a = _.m(_.xZ);
				var b = _.m(_.Hu);
				_.Kg((c) => {
					for (let d of this.menu().zq()) {
						if (!d.qm()) continue;
						let e = _.wZ(a, d).subscribe(() => {
							b.lb();
						});
						c(() => {
							e.unsubscribe();
						});
					}
				});
			}
		};
		$5.J = function(a) {
			return new (a || $5)();
		};
		$5.ka = _.u({
			type: $5,
			da: [["sdui-menu-component"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.GEa, $Ld, 5, _.Jf)(b.Aoa, _.SZ, 5);
				}
				if (a & 2) {
					_.ki(2);
				}
			},
			inputs: {
				xb: [1, "comp"],
				id: [1, "id"],
				triggerId: [1, "triggerId"]
			},
			features: [_.yi([{
				Da: _.kP,
				zb: $5
			}]), _.mh([_.cR])],
			ha: 4,
			ia: 2,
			la: [
				["menuElement", ""],
				[
					"role",
					"menu",
					1,
					"sdui-menu-component",
					3,
					"keydown",
					"id"
				],
				[
					"sdui-component",
					"",
					3,
					"comp"
				],
				[3, "comp"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 1, 0), _.J("keydown", function(c) {
						return b.nKb(c);
					}), _.Ah(2, IKd, 1, 1, null, null, _.yh), _.H();
				}
				if (a & 2) {
					_.E("id", b.id()), _.wh("aria-labelledby", b.triggerId()), _.y(2), _.Bh(b.menu().zq());
				}
			},
			dependencies: [
				_.tz,
				_.SZ,
				_.zZ
			],
			styles: ["[_nghost-%COMP%]   .sdui--background-color[_ngcontent-%COMP%]{background-color:var(--sdui--background-color)!important}[_nghost-%COMP%]   .sdui--background-color--hover[_ngcontent-%COMP%]:hover{background-color:var(--sdui--background-color--hover)!important}[_nghost-%COMP%]   .sdui--background-color--focus[_ngcontent-%COMP%]:focus-visible{background-color:var(--sdui--background-color--focus)!important}[_nghost-%COMP%]   .sdui--border-bottom-color[_ngcontent-%COMP%]{border-bottom-color:var(--sdui--border-bottom-color)!important}[_nghost-%COMP%]   .sdui--border-bottom-color--hover[_ngcontent-%COMP%]:hover{border-bottom-color:var(--sdui--border-bottom-color--hover)!important}[_nghost-%COMP%]   .sdui--border-bottom-color--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-color:var(--sdui--border-bottom-color--focus)!important}[_nghost-%COMP%]   .sdui--border-bottom-style[_ngcontent-%COMP%]{border-bottom-style:var(--sdui--border-bottom-style)!important}[_nghost-%COMP%]   .sdui--border-bottom-style--hover[_ngcontent-%COMP%]:hover{border-bottom-style:var(--sdui--border-bottom-style--hover)!important}[_nghost-%COMP%]   .sdui--border-bottom-style--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-style:var(--sdui--border-bottom-style--focus)!important}[_nghost-%COMP%]   .sdui--border-bottom-width[_ngcontent-%COMP%]{border-bottom-width:var(--sdui--border-bottom-width)!important}[_nghost-%COMP%]   .sdui--border-bottom-width--hover[_ngcontent-%COMP%]:hover{border-bottom-width:var(--sdui--border-bottom-width--hover)!important}[_nghost-%COMP%]   .sdui--border-bottom-width--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-width:var(--sdui--border-bottom-width--focus)!important}[_nghost-%COMP%]   .sdui--border-left-color[_ngcontent-%COMP%]{border-left-color:var(--sdui--border-left-color)!important}[_nghost-%COMP%]   .sdui--border-left-color--hover[_ngcontent-%COMP%]:hover{border-left-color:var(--sdui--border-left-color--hover)!important}[_nghost-%COMP%]   .sdui--border-left-color--focus[_ngcontent-%COMP%]:focus-visible{border-left-color:var(--sdui--border-left-color--focus)!important}[_nghost-%COMP%]   .sdui--border-left-style[_ngcontent-%COMP%]{border-left-style:var(--sdui--border-left-style)!important}[_nghost-%COMP%]   .sdui--border-left-style--hover[_ngcontent-%COMP%]:hover{border-left-style:var(--sdui--border-left-style--hover)!important}[_nghost-%COMP%]   .sdui--border-left-style--focus[_ngcontent-%COMP%]:focus-visible{border-left-style:var(--sdui--border-left-style--focus)!important}[_nghost-%COMP%]   .sdui--border-left-width[_ngcontent-%COMP%]{border-left-width:var(--sdui--border-left-width)!important}[_nghost-%COMP%]   .sdui--border-left-width--hover[_ngcontent-%COMP%]:hover{border-left-width:var(--sdui--border-left-width--hover)!important}[_nghost-%COMP%]   .sdui--border-left-width--focus[_ngcontent-%COMP%]:focus-visible{border-left-width:var(--sdui--border-left-width--focus)!important}[_nghost-%COMP%]   .sdui--border-right-color[_ngcontent-%COMP%]{border-right-color:var(--sdui--border-right-color)!important}[_nghost-%COMP%]   .sdui--border-right-color--hover[_ngcontent-%COMP%]:hover{border-right-color:var(--sdui--border-right-color--hover)!important}[_nghost-%COMP%]   .sdui--border-right-color--focus[_ngcontent-%COMP%]:focus-visible{border-right-color:var(--sdui--border-right-color--focus)!important}[_nghost-%COMP%]   .sdui--border-right-style[_ngcontent-%COMP%]{border-right-style:var(--sdui--border-right-style)!important}[_nghost-%COMP%]   .sdui--border-right-style--hover[_ngcontent-%COMP%]:hover{border-right-style:var(--sdui--border-right-style--hover)!important}[_nghost-%COMP%]   .sdui--border-right-style--focus[_ngcontent-%COMP%]:focus-visible{border-right-style:var(--sdui--border-right-style--focus)!important}[_nghost-%COMP%]   .sdui--border-right-width[_ngcontent-%COMP%]{border-right-width:var(--sdui--border-right-width)!important}[_nghost-%COMP%]   .sdui--border-right-width--hover[_ngcontent-%COMP%]:hover{border-right-width:var(--sdui--border-right-width--hover)!important}[_nghost-%COMP%]   .sdui--border-right-width--focus[_ngcontent-%COMP%]:focus-visible{border-right-width:var(--sdui--border-right-width--focus)!important}[_nghost-%COMP%]   .sdui--border-top-color[_ngcontent-%COMP%]{border-top-color:var(--sdui--border-top-color)!important}[_nghost-%COMP%]   .sdui--border-top-color--hover[_ngcontent-%COMP%]:hover{border-top-color:var(--sdui--border-top-color--hover)!important}[_nghost-%COMP%]   .sdui--border-top-color--focus[_ngcontent-%COMP%]:focus-visible{border-top-color:var(--sdui--border-top-color--focus)!important}[_nghost-%COMP%]   .sdui--border-top-style[_ngcontent-%COMP%]{border-top-style:var(--sdui--border-top-style)!important}[_nghost-%COMP%]   .sdui--border-top-style--hover[_ngcontent-%COMP%]:hover{border-top-style:var(--sdui--border-top-style--hover)!important}[_nghost-%COMP%]   .sdui--border-top-style--focus[_ngcontent-%COMP%]:focus-visible{border-top-style:var(--sdui--border-top-style--focus)!important}[_nghost-%COMP%]   .sdui--border-top-width[_ngcontent-%COMP%]{border-top-width:var(--sdui--border-top-width)!important}[_nghost-%COMP%]   .sdui--border-top-width--hover[_ngcontent-%COMP%]:hover{border-top-width:var(--sdui--border-top-width--hover)!important}[_nghost-%COMP%]   .sdui--border-top-width--focus[_ngcontent-%COMP%]:focus-visible{border-top-width:var(--sdui--border-top-width--focus)!important}[_nghost-%COMP%]   .sdui--box-shadow[_ngcontent-%COMP%]{box-shadow:var(--sdui--box-shadow)!important}[_nghost-%COMP%]   .sdui--box-shadow--hover[_ngcontent-%COMP%]:hover{box-shadow:var(--sdui--box-shadow--hover)!important}[_nghost-%COMP%]   .sdui--box-shadow--focus[_ngcontent-%COMP%]:focus-visible{box-shadow:var(--sdui--box-shadow--focus)!important}[_nghost-%COMP%]   .sdui--color[_ngcontent-%COMP%]{color:var(--sdui--color)!important}[_nghost-%COMP%]   .sdui--color--hover[_ngcontent-%COMP%]:hover{color:var(--sdui--color--hover)!important}[_nghost-%COMP%]   .sdui--color--focus[_ngcontent-%COMP%]:focus-visible{color:var(--sdui--color--focus)!important}[_nghost-%COMP%]   .sdui--cursor[_ngcontent-%COMP%]{cursor:var(--sdui--cursor)!important}[_nghost-%COMP%]   .sdui--cursor--hover[_ngcontent-%COMP%]:hover{cursor:var(--sdui--cursor--hover)!important}[_nghost-%COMP%]   .sdui--cursor--focus[_ngcontent-%COMP%]:focus-visible{cursor:var(--sdui--cursor--focus)!important}[_nghost-%COMP%]   .sdui--outline-color[_ngcontent-%COMP%]{outline-color:var(--sdui--outline-color)!important}[_nghost-%COMP%]   .sdui--outline-color--hover[_ngcontent-%COMP%]:hover{outline-color:var(--sdui--outline-color--hover)!important}[_nghost-%COMP%]   .sdui--outline-color--focus[_ngcontent-%COMP%]:focus-visible{outline-color:var(--sdui--outline-color--focus)!important}[_nghost-%COMP%]   .sdui--outline-style[_ngcontent-%COMP%]{outline-style:var(--sdui--outline-style)!important}[_nghost-%COMP%]   .sdui--outline-style--hover[_ngcontent-%COMP%]:hover{outline-style:var(--sdui--outline-style--hover)!important}[_nghost-%COMP%]   .sdui--outline-style--focus[_ngcontent-%COMP%]:focus-visible{outline-style:var(--sdui--outline-style--focus)!important}[_nghost-%COMP%]   .sdui--outline-width[_ngcontent-%COMP%]{outline-width:var(--sdui--outline-width)!important}[_nghost-%COMP%]   .sdui--outline-width--hover[_ngcontent-%COMP%]:hover{outline-width:var(--sdui--outline-width--hover)!important}[_nghost-%COMP%]   .sdui--outline-width--focus[_ngcontent-%COMP%]:focus-visible{outline-width:var(--sdui--outline-width--focus)!important}[_nghost-%COMP%]   .sdui--text-shadow[_ngcontent-%COMP%]{text-shadow:var(--sdui--text-shadow)!important}[_nghost-%COMP%]   .sdui--text-shadow--hover[_ngcontent-%COMP%]:hover{text-shadow:var(--sdui--text-shadow--hover)!important}[_nghost-%COMP%]   .sdui--text-shadow--focus[_ngcontent-%COMP%]:focus-visible{text-shadow:var(--sdui--text-shadow--focus)!important}.sdui-menu-component[_ngcontent-%COMP%]{box-shadow:0 3px 1px -2px rgba(0,0,0,.2),0 2px 2px 0 rgba(0,0,0,.14),0 1px 5px 0 rgba(0,0,0,.12);background-color:var(--cm-sys-color-surface-elevation,#fff);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-flow:column nowrap;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-flow:column nowrap;flex-flow:column nowrap}\n/*# sourceMappingURL=menu_component.css.map */"]
		});
		var a6 = class {
			constructor() {
				this.overlay = _.m(_.EB);
				this.closed = this.H = new _.Wg();
				this.Acb = _.Li.required();
				this.O_ = _.W(() => new _.yB(this.pna(), this.Acb()));
				this.Nb = _.M(null);
				_.Kg((a) => {
					this.F();
					a(() => {
						var b = this.Nb();
						if (b) {
							b.dispose(), this.Nb.set(null);
						}
					});
				});
			}
			open() {
				var a = this.Nb();
				if (!a) {
					a = this.overlay.create(this.F()), this.Nb.set(a);
				}
				if (!a.Ug()) {
					a.attach(this.O_());
				}
			}
			close() {
				var a;
				if (!((a = this.Nb()) == null)) {
					a.detach();
				}
				this.H.next();
			}
			Ba() {
				var a;
				if (!((a = this.Nb()) == null)) {
					a.dispose();
				}
			}
		};
		a6.J = function(a) {
			return new (a || a6)();
		};
		a6.Oa = _.We({
			type: a6,
			inputs: { Acb: [1, "triggerViewContainer"] }
		});
		var bMd = function(a, b) {
			b.kN().pipe(_.dh(a.closed), _.Ak(a.ub)).subscribe((c) => {
				c = _.Kl(c);
				var d = a.I1();
				if (!(d && (c === d || d.contains(c)))) {
					a.close();
				}
			});
		};
		var b6 = class extends a6 {
			constructor() {
				super(...arguments);
				this.ub = _.m(_.ag);
				this.xb = _.Li.required();
				this.id = _.Li.required();
				this.eNb = _.Li.required();
				this.triggerId = _.Li.required();
				this.I1 = _.Li.required();
				this.kKb = _.Ni($5);
				this.pna = _.Ni.required("menuOverlay", { read: _.Zh });
				this.F = _.W(() => {
					var a = _.qB(_.rB(_.tB(_.vB(this.overlay.position(), this.I1())), false), CLd);
					return new _.dm({
						uf: false,
						yg: a,
						zj: this.overlay.A.A()
					});
				});
				this.mKb = (a) => {
					var b = a.key;
					if (b === "Escape" || b === "Tab") {
						a.preventDefault();
						this.close();
					}
				};
			}
			toggle() {
				var a = this.Nb();
				if (a && a.Ug()) {
					this.close();
				} else {
					this.open();
				}
			}
			open() {
				super.open();
				this.I1().setAttribute("aria-expanded", "true");
				var a = this.Nb();
				if (a) {
					bMd(this, a);
				}
			}
			close() {
				super.close();
				this.I1().setAttribute("aria-expanded", "false");
				var a;
				if (!((a = this.I1()) == null)) {
					a.focus();
				}
			}
		};
		b6.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(b6)))(b || b6);
			};
		})();
		b6.ka = _.u({
			type: b6,
			da: [["sdui-menu-overlay"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.kKb, $5, 5)(b.pna, aMd, 5, _.Zh);
				}
				if (a & 2) {
					_.ki(2);
				}
			},
			inputs: {
				xb: [1, "comp"],
				id: [1, "id"],
				eNb: [1, "propertyOverrides"],
				triggerId: [1, "triggerId"],
				I1: [1, "triggerButtonElement"]
			},
			features: [_.nh],
			ha: 3,
			ia: 1,
			la: [
				["menuOverlay", ""],
				[
					"sdui-component",
					"",
					3,
					"comp"
				],
				[
					3,
					"keydown",
					"comp",
					"id",
					"triggerId"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Gh(0, 1), _.z(1, KKd, 1, 3, "ng-template", null, 0, _.Ii), _.Hh();
				}
				if (a & 2) {
					_.E("comp", b.xb());
				}
			},
			dependencies: [
				_.tz,
				_.HB,
				_.zZ,
				$5
			],
			styles: ["[_nghost-%COMP%]{display:contents}\n/*# sourceMappingURL=menu_overlay.css.map */"]
		});
		var c6 = class {
			constructor() {
				this.F = new WeakMap();
				this.A = new WeakMap();
			}
			q8a(a, b) {
				b.setAttribute("aria-haspopup", "menu");
				this.F.set(a, b);
			}
			lYa(a) {
				var b = this.A.get(a);
				var c = this.F.get(a);
				if (b) {
					b.destroy(), this.A.delete(a);
				}
				if (c) {
					c.removeAttribute("aria-haspopup"), c.removeAttribute("aria-controls"), this.F.delete(a);
				}
			}
			toggle(a, b) {
				var c = this.A.get(a);
				var d = a.Sb();
				if (d != null && _.Dr(d, _.FFc, 1018, _.CY)) {
					if (!c) {
						c = this.F.get(a);
						if (!c) return;
						c.setAttribute("aria-controls", _.NX(this.E_(c.id)));
						let e = _.Fu(b, b6);
						e.zk("comp", d);
						e.zk("propertyOverrides", _.AY(d));
						e.zk("triggerViewContainer", b);
						e.zk("triggerButtonElement", c);
						e.zk("id", this.E_(c.id));
						e.zk("triggerId", c.id);
						this.A.set(a, e);
						c = e;
					}
					c.zk("comp", a.Sb());
					c.instance.toggle();
				}
			}
			E_(a) {
				return `menu-overlay-${a}`;
			}
		};
		c6.J = function(a) {
			return new (a || c6)();
		};
		c6.sa = _.Cd({
			token: c6,
			factory: c6.J,
			wa: "root"
		});
		var d6 = class {
			constructor() {
				this.xb = _.Li.required();
				this.id = _.W(() => this.xb().getId() || _.iu());
				this.dialog = _.W(() => {
					var c = this.xb();
					return _.fj(c, _.NFc, 1013, _.CY);
				});
				this.Lg = this.oj = this.DQ = _.Ni("dialogContainerElement", Object.assign({}, {}, { read: _.Jf }));
				var a = _.m(_.xZ);
				var b = _.m(_.Hu);
				_.Kg((c) => {
					var d = _.wZ(a, this.xb()).subscribe(() => {
						b.lb();
					});
					c(() => {
						d.unsubscribe();
					});
				});
			}
		};
		d6.J = function(a) {
			return new (a || d6)();
		};
		d6.ka = _.u({
			type: d6,
			da: [["sdui-dialog-component"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.DQ, cMd, 5, _.Jf);
				}
				if (a & 2) {
					_.ki();
				}
			},
			inputs: { xb: [1, "comp"] },
			features: [_.yi([{
				Da: _.kP,
				zb: d6
			}]), _.mh([_.cR])],
			ha: 5,
			ia: 3,
			la: [
				["dialogContainerElement", ""],
				[1, "sdui-dialog-wrapper"],
				[
					"sdui-component",
					"",
					3,
					"comp"
				],
				[
					1,
					"sdui-dialog-content",
					3,
					"comp"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Th(0), _.F(1, "div", 1)(2, "div", 2, 0), _.B(4, MKd, 1, 1, "sdui-component", 3), _.H()();
				}
				if (a & 2) {
					a = b.xb(), b = _.Uh(b.dialog()), _.y(2), _.E("comp", a), _.y(2), _.C(b.Fe() ? 4 : -1);
				}
			},
			dependencies: [
				_.tz,
				_.LZ,
				_.zZ,
				M5
			],
			styles: ["[_nghost-%COMP%]   .sdui--background-color[_ngcontent-%COMP%]{background-color:var(--sdui--background-color)!important}[_nghost-%COMP%]   .sdui--background-color--hover[_ngcontent-%COMP%]:hover{background-color:var(--sdui--background-color--hover)!important}[_nghost-%COMP%]   .sdui--background-color--focus[_ngcontent-%COMP%]:focus-visible{background-color:var(--sdui--background-color--focus)!important}[_nghost-%COMP%]   .sdui--border-bottom-color[_ngcontent-%COMP%]{border-bottom-color:var(--sdui--border-bottom-color)!important}[_nghost-%COMP%]   .sdui--border-bottom-color--hover[_ngcontent-%COMP%]:hover{border-bottom-color:var(--sdui--border-bottom-color--hover)!important}[_nghost-%COMP%]   .sdui--border-bottom-color--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-color:var(--sdui--border-bottom-color--focus)!important}[_nghost-%COMP%]   .sdui--border-bottom-style[_ngcontent-%COMP%]{border-bottom-style:var(--sdui--border-bottom-style)!important}[_nghost-%COMP%]   .sdui--border-bottom-style--hover[_ngcontent-%COMP%]:hover{border-bottom-style:var(--sdui--border-bottom-style--hover)!important}[_nghost-%COMP%]   .sdui--border-bottom-style--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-style:var(--sdui--border-bottom-style--focus)!important}[_nghost-%COMP%]   .sdui--border-bottom-width[_ngcontent-%COMP%]{border-bottom-width:var(--sdui--border-bottom-width)!important}[_nghost-%COMP%]   .sdui--border-bottom-width--hover[_ngcontent-%COMP%]:hover{border-bottom-width:var(--sdui--border-bottom-width--hover)!important}[_nghost-%COMP%]   .sdui--border-bottom-width--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-width:var(--sdui--border-bottom-width--focus)!important}[_nghost-%COMP%]   .sdui--border-left-color[_ngcontent-%COMP%]{border-left-color:var(--sdui--border-left-color)!important}[_nghost-%COMP%]   .sdui--border-left-color--hover[_ngcontent-%COMP%]:hover{border-left-color:var(--sdui--border-left-color--hover)!important}[_nghost-%COMP%]   .sdui--border-left-color--focus[_ngcontent-%COMP%]:focus-visible{border-left-color:var(--sdui--border-left-color--focus)!important}[_nghost-%COMP%]   .sdui--border-left-style[_ngcontent-%COMP%]{border-left-style:var(--sdui--border-left-style)!important}[_nghost-%COMP%]   .sdui--border-left-style--hover[_ngcontent-%COMP%]:hover{border-left-style:var(--sdui--border-left-style--hover)!important}[_nghost-%COMP%]   .sdui--border-left-style--focus[_ngcontent-%COMP%]:focus-visible{border-left-style:var(--sdui--border-left-style--focus)!important}[_nghost-%COMP%]   .sdui--border-left-width[_ngcontent-%COMP%]{border-left-width:var(--sdui--border-left-width)!important}[_nghost-%COMP%]   .sdui--border-left-width--hover[_ngcontent-%COMP%]:hover{border-left-width:var(--sdui--border-left-width--hover)!important}[_nghost-%COMP%]   .sdui--border-left-width--focus[_ngcontent-%COMP%]:focus-visible{border-left-width:var(--sdui--border-left-width--focus)!important}[_nghost-%COMP%]   .sdui--border-right-color[_ngcontent-%COMP%]{border-right-color:var(--sdui--border-right-color)!important}[_nghost-%COMP%]   .sdui--border-right-color--hover[_ngcontent-%COMP%]:hover{border-right-color:var(--sdui--border-right-color--hover)!important}[_nghost-%COMP%]   .sdui--border-right-color--focus[_ngcontent-%COMP%]:focus-visible{border-right-color:var(--sdui--border-right-color--focus)!important}[_nghost-%COMP%]   .sdui--border-right-style[_ngcontent-%COMP%]{border-right-style:var(--sdui--border-right-style)!important}[_nghost-%COMP%]   .sdui--border-right-style--hover[_ngcontent-%COMP%]:hover{border-right-style:var(--sdui--border-right-style--hover)!important}[_nghost-%COMP%]   .sdui--border-right-style--focus[_ngcontent-%COMP%]:focus-visible{border-right-style:var(--sdui--border-right-style--focus)!important}[_nghost-%COMP%]   .sdui--border-right-width[_ngcontent-%COMP%]{border-right-width:var(--sdui--border-right-width)!important}[_nghost-%COMP%]   .sdui--border-right-width--hover[_ngcontent-%COMP%]:hover{border-right-width:var(--sdui--border-right-width--hover)!important}[_nghost-%COMP%]   .sdui--border-right-width--focus[_ngcontent-%COMP%]:focus-visible{border-right-width:var(--sdui--border-right-width--focus)!important}[_nghost-%COMP%]   .sdui--border-top-color[_ngcontent-%COMP%]{border-top-color:var(--sdui--border-top-color)!important}[_nghost-%COMP%]   .sdui--border-top-color--hover[_ngcontent-%COMP%]:hover{border-top-color:var(--sdui--border-top-color--hover)!important}[_nghost-%COMP%]   .sdui--border-top-color--focus[_ngcontent-%COMP%]:focus-visible{border-top-color:var(--sdui--border-top-color--focus)!important}[_nghost-%COMP%]   .sdui--border-top-style[_ngcontent-%COMP%]{border-top-style:var(--sdui--border-top-style)!important}[_nghost-%COMP%]   .sdui--border-top-style--hover[_ngcontent-%COMP%]:hover{border-top-style:var(--sdui--border-top-style--hover)!important}[_nghost-%COMP%]   .sdui--border-top-style--focus[_ngcontent-%COMP%]:focus-visible{border-top-style:var(--sdui--border-top-style--focus)!important}[_nghost-%COMP%]   .sdui--border-top-width[_ngcontent-%COMP%]{border-top-width:var(--sdui--border-top-width)!important}[_nghost-%COMP%]   .sdui--border-top-width--hover[_ngcontent-%COMP%]:hover{border-top-width:var(--sdui--border-top-width--hover)!important}[_nghost-%COMP%]   .sdui--border-top-width--focus[_ngcontent-%COMP%]:focus-visible{border-top-width:var(--sdui--border-top-width--focus)!important}[_nghost-%COMP%]   .sdui--box-shadow[_ngcontent-%COMP%]{box-shadow:var(--sdui--box-shadow)!important}[_nghost-%COMP%]   .sdui--box-shadow--hover[_ngcontent-%COMP%]:hover{box-shadow:var(--sdui--box-shadow--hover)!important}[_nghost-%COMP%]   .sdui--box-shadow--focus[_ngcontent-%COMP%]:focus-visible{box-shadow:var(--sdui--box-shadow--focus)!important}[_nghost-%COMP%]   .sdui--color[_ngcontent-%COMP%]{color:var(--sdui--color)!important}[_nghost-%COMP%]   .sdui--color--hover[_ngcontent-%COMP%]:hover{color:var(--sdui--color--hover)!important}[_nghost-%COMP%]   .sdui--color--focus[_ngcontent-%COMP%]:focus-visible{color:var(--sdui--color--focus)!important}[_nghost-%COMP%]   .sdui--cursor[_ngcontent-%COMP%]{cursor:var(--sdui--cursor)!important}[_nghost-%COMP%]   .sdui--cursor--hover[_ngcontent-%COMP%]:hover{cursor:var(--sdui--cursor--hover)!important}[_nghost-%COMP%]   .sdui--cursor--focus[_ngcontent-%COMP%]:focus-visible{cursor:var(--sdui--cursor--focus)!important}[_nghost-%COMP%]   .sdui--outline-color[_ngcontent-%COMP%]{outline-color:var(--sdui--outline-color)!important}[_nghost-%COMP%]   .sdui--outline-color--hover[_ngcontent-%COMP%]:hover{outline-color:var(--sdui--outline-color--hover)!important}[_nghost-%COMP%]   .sdui--outline-color--focus[_ngcontent-%COMP%]:focus-visible{outline-color:var(--sdui--outline-color--focus)!important}[_nghost-%COMP%]   .sdui--outline-style[_ngcontent-%COMP%]{outline-style:var(--sdui--outline-style)!important}[_nghost-%COMP%]   .sdui--outline-style--hover[_ngcontent-%COMP%]:hover{outline-style:var(--sdui--outline-style--hover)!important}[_nghost-%COMP%]   .sdui--outline-style--focus[_ngcontent-%COMP%]:focus-visible{outline-style:var(--sdui--outline-style--focus)!important}[_nghost-%COMP%]   .sdui--outline-width[_ngcontent-%COMP%]{outline-width:var(--sdui--outline-width)!important}[_nghost-%COMP%]   .sdui--outline-width--hover[_ngcontent-%COMP%]:hover{outline-width:var(--sdui--outline-width--hover)!important}[_nghost-%COMP%]   .sdui--outline-width--focus[_ngcontent-%COMP%]:focus-visible{outline-width:var(--sdui--outline-width--focus)!important}[_nghost-%COMP%]   .sdui--text-shadow[_ngcontent-%COMP%]{text-shadow:var(--sdui--text-shadow)!important}[_nghost-%COMP%]   .sdui--text-shadow--hover[_ngcontent-%COMP%]:hover{text-shadow:var(--sdui--text-shadow--hover)!important}[_nghost-%COMP%]   .sdui--text-shadow--focus[_ngcontent-%COMP%]:focus-visible{text-shadow:var(--sdui--text-shadow--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]{background-color:light-dark(var(--sdui-sys-color-surface),var(--sdui-sys-color-surface-container-low));border-radius:8px;box-shadow:0 1px 2px 0 light-dark(rgba(60,64,67,.3),rgba(0,0,0,.3)),0 2px 6px 2px light-dark(rgba(60,64,67,.15),rgba(0,0,0,.15)),inset 0 0 0 1px light-dark(transparent,#3c4043);-moz-box-sizing:border-box;box-sizing:border-box}"]
		});
		var e6 = class extends a6 {
			constructor() {
				super();
				this.ub = _.m(_.ag);
				this.xb = _.Li.required();
				this.ariaModal = _.V(true);
				this.uf = _.V(true);
				this.JC = _.V();
				this.E8a = _.V(false);
				this.pna = _.Ni.required("dialogOverlay", { read: _.Zh });
				this.qYa = _.Ni.required(d6, { read: _.Jf });
				this.Eqa = _.Ni.required(_.JA);
				this.A = null;
				this.ariaLabel = _.W(() => {
					var a;
					return ((a = _.BY(this.xb())) == null ? undefined : a.Bl()) || null;
				});
				this.lxb = _.W(() => {
					var a;
					return _.NX(((a = _.BY(this.xb())) == null ? undefined : _.l(a, 4)) || null);
				});
				this.yg = _.W(() => this.E8a() ? _.qB(_.rB(_.vB(this.overlay.position(), this.JC()), false), CLd) : _.xib(_.uB(_.U8b(this.overlay.position()))));
				this.F = _.W(() => new _.dm({
					uf: this.uf(),
					yg: this.yg(),
					zj: this.overlay.A.block()
				}));
				_.cj({ write: (a) => {
					var b = this.Nb();
					if (b) {
						var c = new ResizeObserver(() => {
							b.Cj();
						});
						c.observe(b.Nn);
						a(() => {
							c.disconnect();
						});
					}
				} });
			}
			open() {
				if (!this.JC()) {
					this.A = document.activeElement;
				}
				super.open();
				setTimeout(() => {
					if (!_.HA(this.Eqa().qh)) {
						var a = this.qYa().nativeElement;
						let b = a.querySelector("h1, h2, h3, h4, h5, h6, [role=\"heading\"]");
						if (b instanceof HTMLElement || b instanceof SVGElement) {
							b.hasAttribute("tabindex") || b.setAttribute("tabindex", "-1"), b.focus();
						} else {
							a.focus();
						}
					}
					if (a = this.Nb()) {
						_.Ff(a.Au(), a.kx().pipe(_.Gf((b) => b.key === "Escape"))).pipe(_.dh(this.closed), _.Ak(this.ub)).subscribe(() => {
							this.close();
						});
					}
				});
			}
			close() {
				super.close();
				var a;
				if (!((a = this.JC() || this.A) == null)) {
					a.focus();
				}
				this.A = null;
			}
		};
		e6.J = function(a) {
			return new (a || e6)();
		};
		e6.ka = _.u({
			type: e6,
			da: [["sdui-dialog-overlay"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.pna, dMd, 5, _.Zh)(b.qYa, d6, 5, _.Jf)(b.Eqa, _.JA, 5);
				}
				if (a & 2) {
					_.ki(3);
				}
			},
			inputs: {
				xb: [1, "comp"],
				ariaModal: [1, "ariaModal"],
				uf: [1, "hasBackdrop"],
				JC: [1, "triggerElement"],
				E8a: [1, "renderRelativeToTrigger"]
			},
			features: [_.nh],
			ha: 2,
			ia: 0,
			la: [
				["dialogOverlay", ""],
				[
					"cdkTrapFocus",
					"",
					1,
					"sdui-dialog-overlay-container"
				],
				[
					"role",
					"dialog",
					"tabindex",
					"-1",
					3,
					"comp"
				]
			],
			template: function(a) {
				if (a & 1) {
					_.z(0, NKd, 2, 4, "ng-template", null, 0, _.Ii);
				}
			},
			dependencies: [
				_.TA,
				_.JA,
				_.tz,
				_.HB,
				d6
			],
			styles: ["[_nghost-%COMP%]{display:contents}.sdui-dialog-overlay-container[_ngcontent-%COMP%]{max-width:80vw;min-width:0}"]
		});
		var f6 = class {
			constructor() {
				this.A = [];
				this.F = null;
			}
			open(a, b) {
				var c = a.Sb();
				if (c) {
					var d = b;
					try {
						var e = _.Fu(b, e6);
						if (e.location.nativeElement.isConnected) {
							this.F = b;
						} else {
							e.destroy(), e = undefined;
						}
					} catch (p) {
						e = undefined;
					}
					var f;
					if (!e && ((f = this.F) == null ? 0 : f.element.nativeElement.isConnected)) {
						e = _.Fu(this.F, e6);
						d = this.F;
					}
					if (!e) throw Error("Bi");
					e.zk("comp", c);
					e.zk("triggerViewContainer", d);
					if (_.vn(a, 1e3)) {
						e.zk("hasBackdrop", _.Pm(a, 1e3));
					}
					if (_.vn(a, 12)) {
						e.zk("ariaModal", _.Pm(a, 12));
					}
					if (_.vn(a, 1001)) {
						e.zk("renderRelativeToTrigger", _.Pm(a, 1001));
					}
					var g;
					var k;
					if (a = (g = b.Pa.get(_.kP, null)) == null ? undefined : (k = g.Lg()) == null ? undefined : k.nativeElement) {
						e.zk("triggerElement", a);
					}
					e.Hc(() => {
						_.Ca(this.A, e);
					});
					this.A.push(e);
					try {
						e.instance.open();
						e.instance.closed.subscribe(() => {
							e.destroy();
						});
					} catch (p) {
						throw e.destroy(), p;
					}
				}
			}
			close() {
				var a = this.A[this.A.length - 1];
				if (a) {
					a.instance.close();
				}
			}
		};
		f6.J = function(a) {
			return new (a || f6)();
		};
		f6.sa = _.Cd({
			token: f6,
			factory: f6.J,
			wa: "root"
		});
		var eMd = new _.hP("45723900");
		var g6 = class {
			constructor() {
				this.A = _.m(f6);
				this.F = _.m(_.rC);
				this.Fr = 4;
			}
			Zg(a) {
				a = _.fj(a, _.AEc, 4, _.FY);
				if (_.cP(eMd)) this.A.close();
				else {
					a = a == null ? undefined : a.getId();
					let b;
					if (!((b = mLd(this.F, a)) == null)) {
						b.close();
					}
				}
			}
			reset() {}
		};
		g6.J = function(a) {
			return new (a || g6)();
		};
		g6.sa = _.Cd({
			token: g6,
			factory: g6.J
		});
		var h6 = class {
			constructor() {
				this.A = _.m(_.SC);
				this.Mj = _.m(_.SA);
			}
			copy(a) {
				if (a !== "") {
					a = this.A.copy(a), _.RA(this.Mj, a ? "Copied" : "Copy failed");
				}
			}
		};
		h6.J = function(a) {
			return new (a || h6)();
		};
		h6.sa = _.Cd({
			token: h6,
			factory: h6.J,
			wa: "root"
		});
		var i6 = class {
			constructor() {
				this.A = _.m(h6);
				this.Fr = 1;
			}
			Zg(a) {
				var b;
				if (a = (b = _.fj(a, _.FEc, 1, _.FY)) == null ? undefined : b.getText()) {
					this.A.copy(a);
				}
			}
			reset() {}
		};
		i6.J = function(a) {
			return new (a || i6)();
		};
		i6.sa = _.Cd({
			token: i6,
			factory: i6.J
		});
		var fMd = function(a, b, c = "file.txt", d = false) {
			_.x(function* () {
				var e = b ? atob(a) : a;
				if (d) e = new TextEncoder().encode(e);
				else {
					var f = new Uint8Array(e.length);
					for (let g = 0; g < e.length; g++) f[g] = e[g].charCodeAt(0);
					e = f.buffer;
				}
				f = c.lastIndexOf(".");
				return _.WL.download(new Blob([e], { type: OKd(f === -1 ? "" : c.substr(f + 1).toLowerCase()) }), c);
			});
		};
		var j6 = class {};
		j6.J = function(a) {
			return new (a || j6)();
		};
		j6.sa = _.Cd({
			token: j6,
			factory: j6.J,
			wa: "root"
		});
		var gMd = function(a, b) {
			_.x(function* () {
				if (_.zn(b, 1)) {
					var c = b.getUrl();
					try {
						let e = yield (yield fetch(c)).blob();
						var d;
						if (!(d = b.getFileName())) try {
							let f = new URL(c, document.baseURI).pathname;
							let g = f.lastIndexOf("/");
							d = g !== -1 ? f.substring(g + 1) : "";
						} catch (f) {
							d = "";
						}
						yield _.WL.download(e, d || "file.txt");
					} catch (e) {
						if (c = _.pKc(c)) {
							a.A.navigate(c, { Sq: true });
						}
					}
				}
			});
		};
		var k6 = class {
			constructor() {
				_.m(j6);
				this.A = _.m(_.DZ);
				this.Fr = 10;
			}
			Zg(a) {
				a = _.fj(a, _.HEc, 10, _.FY);
				var b = _.jj(a, _.IEc);
				switch (b) {
					case 5:
						a = _.fj(a, qLd, 5, _.IEc);
						fMd(a.Sb(), _.Pm(a, 3), a.getFileName() || "file.txt", _.Pm(a, 4));
						break;
					case 6:
						gMd(this, _.fj(a, rLd, 6, _.IEc));
						break;
					default: _.sb(b, undefined);
				}
			}
			reset() {}
		};
		k6.J = function(a) {
			return new (a || k6)();
		};
		k6.sa = _.Cd({
			token: k6,
			factory: k6.J
		});
		var hMd = BigInt(Number.MAX_SAFE_INTEGER);
		var iMd = BigInt(Number.MIN_SAFE_INTEGER);
		var kMd = function(a, b) {
			var c = _.jj(b, _.oY);
			switch (c) {
				case 1: return _.ro(new _.qo(), _.Lm(b, _.Ls(b, _.oY, 1)));
				case 2: return _.so(new _.qo(), _.zo(b, 2, _.oY));
				case 3: return _.to(new _.qo(), _.qj(b, 3, _.oY));
				case 4: return _.nBa(new _.qo(), _.Ao(b, 4, _.oY));
				case 5: return _.wo(new _.qo(), jMd(a, _.fj(b, _.nY, 5, _.oY)));
				case 6: return _.vo(new _.qo(), BKd(new _.uo(), oLd(_.fj(b, pLd, 6, _.oY)).map((d) => kMd(a, d))));
				case 7: return b = _.sZ(a.Jb, _.fj(b, _.QX, 7, _.oY)), lMd(a, b);
				default: _.sb(c);
			}
		};
		var jMd = function(a, b) {
			var c = new _.no();
			for (let [d, e] of _.zc(b, 1, nLd).entries()) _.oo(c, d, kMd(a, e));
			return c;
		};
		var lMd = function(a, { value: b, type: c }) {
			switch (c) {
				case "null": return _.ro(new _.qo(), 0);
				case "bool": return _.nBa(new _.qo(), b);
				case "int64":
				case "uint64": return mMd(b, c);
				case "double": return _.so(new _.qo(), b);
				case "string":
				case "duration":
				case "timestamp": return _.to(new _.qo(), b.toString());
				case "bytes": return _.to(new _.qo(), _.Is(b));
				case "list": return _.vo(new _.qo(), BKd(new _.uo(), b.elements.map((d) => lMd(a, d))));
				case "map":
					c = new _.no();
					for (let [d, e] of b.entries()) {
						b = d;
						let f = e;
						if (b.type !== "string") throw Error("Ci`" + b.type);
						_.oo(c, b.value, lMd(a, f));
					}
					return _.wo(new _.qo(), c);
				case "type": return _.to(new _.qo(), b);
				case "error": throw Error("Di");
				default: _.sb(c, undefined);
			}
		};
		var mMd = function(a, b) {
			if (b === "uint64") {
				if (a > hMd) return _.to(new _.qo(), a.toString());
			} else if (a > hMd || a < iMd) return _.to(new _.qo(), a.toString());
			return _.so(new _.qo(), Number(a));
		};
		var l6 = class {
			constructor() {
				this.Jb = _.m(_.tZ);
			}
		};
		l6.J = function(a) {
			return new (a || l6)();
		};
		l6.sa = _.Cd({
			token: l6,
			factory: l6.J
		});
		var nMd = function(a, b) {
			var c;
			var d = (c = b.getId()) != null ? c : "";
			if (!a.A.has(d)) {
				a.A.set(d, new Set());
			}
			var e = a.A.get(d);
			var f = new _.Wg();
			e.add(f);
			f.subscribe({ complete: () => {
				e.delete(f);
				if (e.size === 0) {
					a.A.delete(d);
				}
			} });
			return f;
		};
		var m6 = class {
			constructor() {
				this.F = _.m(l6);
				this.H = _.m(_.xZ);
				this.I = _.m(T5);
				this.Jb = _.m(_.tZ);
				this.A = new Map();
				this.Fr = 6;
				_.m(_.ag).Hc(() => {
					this.reset();
				});
			}
			Zg(a, b, c) {
				a = _.fj(a, _.oFc, 6, _.FY);
				var d;
				var e = {
					intent: _.l(a, 1),
					DOb: _.l(a, 3),
					payload: jMd(this.F, (d = _.Z(a, _.nY, 2)) != null ? d : new _.nY()),
					f8b: _.zc(a, 5, sLd),
					PKb: new Map()
				};
				if (_.zc(a, 4, _.sY).size > 0) {
					e.PKb = new Map(Array.from(_.zc(a, 4, _.sY), ([g, k]) => {
						var p = new _.sY();
						var r = k.getId();
						p = _.Lj(p, 1, r);
						r = p.setValue;
						var v = new _.OX();
						var w = v.setValue;
						var D = this.Jb;
						k = k.getId();
						k = D.signals.get(k)();
						return [g, r.call(p, w.call(v, $Kd(k)))];
					}));
				}
				d = _.mf(undefined);
				if (_.sn(a, vLd, 1e3) && uLd(_.Z(a, vLd, 1e3))) {
					let g = Math.max(0, PKd(tLd(_.Z(a, vLd, 1e3))));
					if (g > 0) {
						d = _.Cf(g);
					}
				}
				var f = nMd(this, a);
				d.pipe(_.ch(() => this.I.A(e)), _.dh(f), _.Tg(() => {
					if (!f.closed) {
						f.complete();
					}
				})).subscribe({
					next: (g) => {
						if (_.sn(g, _.tY, 1) && (_.FJc(this.Jb, _.zc(g, 3, _.OX)), g = _.Z(g, _.tY, 1), !this.H.update(g) && g.JY() === 0)) for (let k of _.mj(g, _.uY, 8, _.oj())) c.Zg(k, b);
					},
					error: () => {}
				});
			}
			reset() {
				var a = Array.from(this.A.values());
				for (let b of a) for (let c of Array.from(b)) c.next(), c.complete();
			}
		};
		m6.J = function(a) {
			return new (a || m6)();
		};
		m6.sa = _.Cd({
			token: m6,
			factory: m6.J
		});
		var n6 = class {
			constructor() {
				this.Fr = 1005;
				this.A = _.m(m6);
			}
			Zg(a) {
				var b = _.fj(a, _.KEc, 1005, _.FY);
				if (b != null && _.zn(b, 1) && (a = this.A, b = _.l(b, 1), !_.ma(b) && a.A.has(b))) {
					a = Array.from(a.A.get(b).values());
					for (let c of a) c.next(), c.complete();
				}
			}
			reset() {}
		};
		n6.J = function(a) {
			return new (a || n6)();
		};
		n6.sa = _.Cd({
			token: n6,
			factory: n6.J
		});
		var o6 = class {
			constructor() {
				this.Fr = 1003;
			}
			Zg(a) {
				a = _.fj(a, _.MEc, 1003, _.FY);
				if (a = _.MX(a == null ? undefined : _.l(a, 1))) {
					let b;
					if (!((b = document.getElementById(a)) == null)) {
						b.focus();
					}
				}
			}
			reset() {}
		};
		o6.J = function(a) {
			return new (a || o6)();
		};
		o6.sa = _.Cd({
			token: o6,
			factory: o6.J
		});
		var p6 = class {
			constructor() {
				this.Jb = _.m(_.tZ);
				this.Fr = 1002;
			}
			Zg(a, b, c) {
				a = _.fj(a, _.XFc, 1002, _.FY);
				if (a != null && _.sn(a, _.QX, 1)) if (_.sZ(this.Jb, _.Z(a, _.QX, 1)).value) for (let d of _.mj(a, _.uY, 2, _.oj())) c.Zg(d, b);
				else for (let d of _.mj(a, _.uY, 3, _.oj())) c.Zg(d, b);
			}
			reset() {}
		};
		p6.J = function(a) {
			return new (a || p6)();
		};
		p6.sa = _.Cd({
			token: p6,
			factory: p6.J
		});
		var q6 = class {
			constructor() {
				this.config = _.m(_.qC);
				this.xb = this.config.dialog;
				this.id = _.M(this.xb.getId() || _.iu());
				this.dialog = _.fj(this.xb, _.NFc, 1013, _.CY);
				this.Lg = this.oj = this.DQ = _.Ni("dialogContainerElement", Object.assign({}, {}, { read: _.Jf }));
				var a = _.m(_.xZ);
				var b = _.m(_.Hu);
				_.Kg((c) => {
					var d = _.wZ(a, this.xb).subscribe(() => {
						b.lb();
					});
					c(() => {
						d.unsubscribe();
					});
				});
				this.mGb = _.m(_.rLc, { optional: true }) === "sdui-viewer";
			}
		};
		q6.J = function(a) {
			return new (a || q6)();
		};
		q6.ka = _.u({
			type: q6,
			da: [["sdui-dialog"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.DQ, oMd, 5, _.Jf);
				}
				if (a & 2) {
					_.ki();
				}
			},
			features: [_.yi([{
				Da: _.kP,
				zb: q6
			}]), _.mh([_.cR])],
			ha: 4,
			ia: 4,
			la: [
				["dialogContainerElement", ""],
				[1, "sdui-dialog-wrapper"],
				[
					"sdui-component",
					"",
					3,
					"comp"
				],
				[
					1,
					"sdui-dialog-content",
					3,
					"comp"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 1)(1, "div", 2, 0), _.B(3, dLd, 1, 1, "sdui-component", 3), _.H()();
				}
				if (a & 2) {
					_.P("sdui-dialog-negative-margins", b.mGb), _.y(), _.E("comp", b.xb), _.y(2), _.C(b.dialog.Fe() ? 3 : -1);
				}
			},
			dependencies: [_.LZ, _.zZ],
			styles: [".sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--background-color[_ngcontent-%COMP%]{background-color:var(--sdui--background-color)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--background-color--hover[_ngcontent-%COMP%]:hover{background-color:var(--sdui--background-color--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--background-color--focus[_ngcontent-%COMP%]:focus-visible{background-color:var(--sdui--background-color--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-color[_ngcontent-%COMP%]{border-bottom-color:var(--sdui--border-bottom-color)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-color--hover[_ngcontent-%COMP%]:hover{border-bottom-color:var(--sdui--border-bottom-color--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-color--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-color:var(--sdui--border-bottom-color--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-style[_ngcontent-%COMP%]{border-bottom-style:var(--sdui--border-bottom-style)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-style--hover[_ngcontent-%COMP%]:hover{border-bottom-style:var(--sdui--border-bottom-style--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-style--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-style:var(--sdui--border-bottom-style--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-width[_ngcontent-%COMP%]{border-bottom-width:var(--sdui--border-bottom-width)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-width--hover[_ngcontent-%COMP%]:hover{border-bottom-width:var(--sdui--border-bottom-width--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-bottom-width--focus[_ngcontent-%COMP%]:focus-visible{border-bottom-width:var(--sdui--border-bottom-width--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-color[_ngcontent-%COMP%]{border-left-color:var(--sdui--border-left-color)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-color--hover[_ngcontent-%COMP%]:hover{border-left-color:var(--sdui--border-left-color--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-color--focus[_ngcontent-%COMP%]:focus-visible{border-left-color:var(--sdui--border-left-color--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-style[_ngcontent-%COMP%]{border-left-style:var(--sdui--border-left-style)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-style--hover[_ngcontent-%COMP%]:hover{border-left-style:var(--sdui--border-left-style--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-style--focus[_ngcontent-%COMP%]:focus-visible{border-left-style:var(--sdui--border-left-style--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-width[_ngcontent-%COMP%]{border-left-width:var(--sdui--border-left-width)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-width--hover[_ngcontent-%COMP%]:hover{border-left-width:var(--sdui--border-left-width--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-left-width--focus[_ngcontent-%COMP%]:focus-visible{border-left-width:var(--sdui--border-left-width--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-color[_ngcontent-%COMP%]{border-right-color:var(--sdui--border-right-color)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-color--hover[_ngcontent-%COMP%]:hover{border-right-color:var(--sdui--border-right-color--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-color--focus[_ngcontent-%COMP%]:focus-visible{border-right-color:var(--sdui--border-right-color--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-style[_ngcontent-%COMP%]{border-right-style:var(--sdui--border-right-style)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-style--hover[_ngcontent-%COMP%]:hover{border-right-style:var(--sdui--border-right-style--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-style--focus[_ngcontent-%COMP%]:focus-visible{border-right-style:var(--sdui--border-right-style--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-width[_ngcontent-%COMP%]{border-right-width:var(--sdui--border-right-width)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-width--hover[_ngcontent-%COMP%]:hover{border-right-width:var(--sdui--border-right-width--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-right-width--focus[_ngcontent-%COMP%]:focus-visible{border-right-width:var(--sdui--border-right-width--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-color[_ngcontent-%COMP%]{border-top-color:var(--sdui--border-top-color)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-color--hover[_ngcontent-%COMP%]:hover{border-top-color:var(--sdui--border-top-color--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-color--focus[_ngcontent-%COMP%]:focus-visible{border-top-color:var(--sdui--border-top-color--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-style[_ngcontent-%COMP%]{border-top-style:var(--sdui--border-top-style)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-style--hover[_ngcontent-%COMP%]:hover{border-top-style:var(--sdui--border-top-style--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-style--focus[_ngcontent-%COMP%]:focus-visible{border-top-style:var(--sdui--border-top-style--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-width[_ngcontent-%COMP%]{border-top-width:var(--sdui--border-top-width)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-width--hover[_ngcontent-%COMP%]:hover{border-top-width:var(--sdui--border-top-width--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--border-top-width--focus[_ngcontent-%COMP%]:focus-visible{border-top-width:var(--sdui--border-top-width--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--box-shadow[_ngcontent-%COMP%]{box-shadow:var(--sdui--box-shadow)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--box-shadow--hover[_ngcontent-%COMP%]:hover{box-shadow:var(--sdui--box-shadow--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--box-shadow--focus[_ngcontent-%COMP%]:focus-visible{box-shadow:var(--sdui--box-shadow--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--color[_ngcontent-%COMP%]{color:var(--sdui--color)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--color--hover[_ngcontent-%COMP%]:hover{color:var(--sdui--color--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--color--focus[_ngcontent-%COMP%]:focus-visible{color:var(--sdui--color--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--cursor[_ngcontent-%COMP%]{cursor:var(--sdui--cursor)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--cursor--hover[_ngcontent-%COMP%]:hover{cursor:var(--sdui--cursor--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--cursor--focus[_ngcontent-%COMP%]:focus-visible{cursor:var(--sdui--cursor--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-color[_ngcontent-%COMP%]{outline-color:var(--sdui--outline-color)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-color--hover[_ngcontent-%COMP%]:hover{outline-color:var(--sdui--outline-color--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-color--focus[_ngcontent-%COMP%]:focus-visible{outline-color:var(--sdui--outline-color--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-style[_ngcontent-%COMP%]{outline-style:var(--sdui--outline-style)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-style--hover[_ngcontent-%COMP%]:hover{outline-style:var(--sdui--outline-style--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-style--focus[_ngcontent-%COMP%]:focus-visible{outline-style:var(--sdui--outline-style--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-width[_ngcontent-%COMP%]{outline-width:var(--sdui--outline-width)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-width--hover[_ngcontent-%COMP%]:hover{outline-width:var(--sdui--outline-width--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--outline-width--focus[_ngcontent-%COMP%]:focus-visible{outline-width:var(--sdui--outline-width--focus)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--text-shadow[_ngcontent-%COMP%]{text-shadow:var(--sdui--text-shadow)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--text-shadow--hover[_ngcontent-%COMP%]:hover{text-shadow:var(--sdui--text-shadow--hover)!important}.sdui-dialog-wrapper[_ngcontent-%COMP%]   .sdui--text-shadow--focus[_ngcontent-%COMP%]:focus-visible{text-shadow:var(--sdui--text-shadow--focus)!important}.sdui-dialog-negative-margins[_ngcontent-%COMP%]{margin:-24px}"]
		});
		var r6 = class {
			constructor() {
				this.A = _.m(f6);
				this.Pa = _.m(_.Xf);
				this.F = _.m(_.rC);
				this.Fr = 2;
			}
			Zg(a, b) {
				a = _.fj(a, _.WFc, 2, _.FY);
				if (_.cP(eMd)) this.A.open(a, b);
				else {
					b = a.Sb();
					var c = _.BY(b);
					let d = (c == null ? undefined : c.Bl()) || null;
					c = _.NX(c == null ? undefined : _.l(c, 4));
					_.In(b, 9);
					this.F.open(q6, {
						Pa: this.Pa,
						data: { dialog: b },
						id: b.getId(),
						ariaLabel: d,
						IK: c,
						ariaModal: _.Pm(a, 12),
						uf: _.vn(a, 1e3) ? _.Pm(a, 1e3) : true
					});
				}
			}
			reset() {}
		};
		r6.J = function(a) {
			return new (a || r6)();
		};
		r6.sa = _.Cd({
			token: r6,
			factory: r6.J
		});
		var s6 = class {
			constructor() {
				this.Jb = _.m(_.tZ);
				this.A = _.m(_.RZ);
				this.Fr = 1004;
			}
			Zg(a) {
				a = xKd(_.fj(a, _.YEc, 1004, _.FY), `ReportFormValidityActionHandler.executeAction called with unsupported action type${_.jj(a, _.FY)}`);
				var b = a.NY();
				a = _.Z(a, _.lY, 2);
				var c = BLd(this.A, b);
				b = true;
				var d = [];
				for (var e of c) {
					c = e.Gu;
					var f = e.Dla;
					let g = e.isValid;
					if (f) {
						_.rZ(this.Jb, f.getName(), _.XX(true));
					}
					f = true;
					if (g) {
						f = _.sZ(this.Jb, g).value;
					}
					if (!f) {
						b = false, d.push(c);
					}
				}
				_.rZ(this.Jb, a.getName(), b ? _.XX(true) : _.XX(false));
				if (!b) {
					e = d.map((g) => `#${_.MX(g)}`).join(", "), (e = document.querySelector(e)) && e.focus();
				}
			}
			reset() {}
		};
		s6.J = function(a) {
			return new (a || s6)();
		};
		s6.sa = _.Cd({
			token: s6,
			factory: s6.J
		});
		var t6 = class {
			constructor() {
				this.Jb = _.m(_.tZ);
				this.Fr = 5;
			}
			Zg(a) {
				var b = _.fj(a, _.pFc, 5, _.FY);
				var c;
				a = b == null ? undefined : (c = _.Z(b, _.sY, 3)) == null ? undefined : c.getId();
				var d;
				var e;
				c = b == null ? undefined : (d = _.Z(b, _.sY, 3)) == null ? undefined : (e = d.getValue()) == null ? undefined : e.getValue();
				if (a && c) {
					_.rZ(this.Jb, a, _.sZ(this.Jb, c));
				}
			}
			reset() {}
		};
		t6.J = function(a) {
			return new (a || t6)();
		};
		t6.sa = _.Cd({
			token: t6,
			factory: t6.J
		});
		var u6 = class {
			constructor() {
				this.A = _.m(c6);
				this.Fr = 12;
			}
			Zg(a, b) {
				a = _.fj(a, _.EY, 12, _.FY);
				this.A.toggle(a, b);
			}
			reset() {}
		};
		u6.J = function(a) {
			return new (a || u6)();
		};
		u6.sa = _.Cd({
			token: u6,
			factory: u6.J
		});
		var v6 = class {
			constructor() {
				this.A = _.m(_.xQ, { optional: true });
				this.Fr = 9;
			}
			Zg(a) {
				if (this.A && (a = _.fj(a, _.rFc, 9, _.FY), a.getType() && a.getName())) {
					var b = {
						type: a.getType(),
						name: a.getName()
					};
					if (_.Yo(a, 3).size > 0) {
						b.metadata = Object.fromEntries(_.Yo(a, 3));
					}
					this.A.sendEvent(b);
				}
			}
			reset() {}
		};
		v6.J = function(a) {
			return new (a || v6)();
		};
		v6.sa = _.Cd({
			token: v6,
			factory: v6.J
		});
		var pMd = class extends _.kZ {
			constructor() {
				super();
				var a;
				var b = (a = _.m(WLd, { optional: true })) != null ? a : [];
				this.A = new Map(b.map((c) => [c.Fr, c]));
				eLd(b);
				_.m(_.ag).Hc(() => {
					this.reset();
				});
			}
			Zg(a, b) {
				var c = _.jj(a, _.FY);
				var d;
				if (!((d = this.A.get(c)) == null)) {
					d.Zg(a, b, this);
				}
			}
			reset() {
				for (let a of this.A.values()) a.reset();
			}
		};
		var qMd = [m6, {
			Da: WLd,
			zb: m6,
			fe: true
		}].concat([
			g6,
			i6,
			k6,
			n6,
			o6,
			p6,
			r6,
			s6,
			t6,
			u6,
			v6
		].map((a) => ({
			Da: WLd,
			Mf: a,
			fe: true
		})));
		var rMd = [
			{
				Da: _.kZ,
				Mf: pMd
			},
			_.lZ,
			_.RZ,
			_.vZ,
			l6,
			_.tZ,
			_.xZ,
			{
				Da: _.yKc,
				zb: c6
			}
		].concat(qMd);
		var sMd = class extends _.cR {
			constructor() {
				super();
				this.Wq = _.V();
				this.nWa = _.V(true);
				var a = _.m(_.kZ);
				var b = _.m(_.xZ);
				var c = _.m(_.tZ);
				var d = _.m(_.Hu);
				var e = _.m(_.$h);
				this.lOb = _.W(() => {
					if (this.Wq()) return ALd(b).pipe(_.eh(() => {
						d.lb();
					}));
				});
				_.Kg((f) => {
					var g = this.Wq();
					if (g) {
						var k = g.A.pipe(_.Gf(LKd), _.uf((p, r) => ({
							role: r === 0 ? "replace" : "update",
							result: p
						}))).subscribe({
							next: ({ role: p, result: r }) => {
								var v = _.Z(r, _.tY, 1);
								switch (p) {
									case "replace":
										b.reset();
										zLd(c);
										_.FJc(c, _.zc(r, 3, _.OX));
										v && (b.tree.A = v, b.F.next());
										break;
									case "update": if (_.FJc(c, _.zc(r, 3, _.OX)), v && !b.update(v) && v.JY() === 0) for (let w of _.mj(v, _.uY, 8, _.oj())) a.Zg(w, e);
								}
							},
							error: () => {}
						});
						f(() => {
							if (_.Qd(this.nWa)) {
								b.reset(), zLd(c);
							}
							a.reset();
							k.unsubscribe();
						});
					}
				});
			}
		};
		sMd.J = function(a) {
			return new (a || sMd)();
		};
		sMd.Oa = _.We({
			type: sMd,
			inputs: {
				Wq: [1, "renderData"],
				nWa: [1, "clearViewerOnRenderDataChange"]
			},
			features: [_.nh]
		});
		_.w6 = class extends sMd {
			constructor() {
				super(...arguments);
				this.logger = null;
				this.veLoggingService = _.m(_.Jk);
			}
		};
		_.w6.prototype.R9 = _.ba(212);
		_.w6.J = (() => {
			var a;
			return function(b) {
				return (a || (a = _.Qe(_.w6)))(b || _.w6);
			};
		})();
		_.w6.ka = _.u({
			type: _.w6,
			da: [["ai-studio-sdui-viewer"]],
			features: [_.yi([
				rMd,
				_.DZ,
				_.BZ,
				{
					Da: T5,
					zb: _.Q5
				},
				{
					Da: _.EZ,
					Mf: V5
				},
				{
					Da: WLd,
					Mf: Y5,
					fe: true
				},
				{
					Da: WLd,
					Mf: Z5,
					fe: true
				},
				{
					Da: _.TJc,
					Mf: U5
				},
				{
					Da: _.dJc,
					zb: P5
				},
				{
					Da: _.yKc,
					zb: c6
				},
				{
					Da: _.jP,
					zb: W5
				},
				{
					Da: _.dR,
					ke: (a) => ({ xNb: a.A() }),
					hg: [O5]
				}
			]), _.nh],
			ha: 2,
			ia: 3,
			la: [[3, "comp"]],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, fLd, 1, 1, "sdui-component", 0), _.Ei(1, "async");
				}
				if (a & 2) {
					let c;
					_.C((c = _.Fi(1, 1, b.lOb())) ? 0 : -1, c);
				}
			},
			dependencies: [_.LZ, _.oz],
			styles: ["[_nghost-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;--cm-sys-type-title-large:600 24px/32px \"Inter Tight\",sans-serif;--ac-sys-color-chart-legend-label:var(--color-v3-text-var);--ac-sys-color-chart-axis-label:var(--chart-axis-label-light);--ac-sys-color-chart-grid:var(--chart-grid-light)}.dark-theme   [_nghost-%COMP%]{--ac-sys-color-chart-axis-label:var(--chart-axis-label-dark);--ac-sys-color-chart-grid:var(--chart-grid-dark)}[_nghost-%COMP%]     datavis-container{color:var(--color-v3-text-var)}[_nghost-%COMP%]     datavis-container .mat-mdc-card.mat-mdc-card, [_nghost-%COMP%]     datavis-container .mat-mdc-card:hover{background:transparent}[_nghost-%COMP%]     datavis-container .mat-mdc-card-header{padding:0}[_nghost-%COMP%]     datavis-container .mat-mdc-card-content{padding:0}  .ai-studio-sdui-viewer-tokens-856794408{--sdui-sys-color-primary:var(--cm-sys-color-primary,#3367d6);--sdui-sys-color-on-primary:var(--cm-sys-color-on-primary,#fff);--sdui-sys-color-inverse-primary:var(--cm-sys-color-primary-inverse,#a1c2fa);--sdui-sys-color-inverse-on-primary:var(--cm-sys-color-on-primary-inverse,#000);--sdui-sys-color-primary-container:var(--cm-sys-color-container-primary,#e8f0fe);--sdui-sys-color-on-primary-container:var(--cm-sys-color-on-container,#000);--sdui-sys-color-secondary:var(--cm-sys-color-surface,#fff);--sdui-sys-color-on-secondary:var(--cm-sys-color-on-surface-variant,rgba(0,0,0,.66));--sdui-sys-color-secondary-container:var(--cm-sys-color-container,#fafafa);--sdui-sys-color-on-secondary-container:var(--cm-sys-color-on-container,#000);--sdui-sys-color-surface:var(--cm-sys-color-surface,#fff);--sdui-sys-color-inverse-surface:var(--cm-sys-color-surface-inverse,#323232);--sdui-sys-color-on-surface:var(--cm-sys-color-on-surface,#000);--sdui-sys-color-inverse-on-surface:var(--cm-sys-color-on-surface-inverse,#fff);--sdui-sys-color-on-surface-variant:var(--cm-sys-color-on-surface-variant,rgba(0,0,0,.66));--sdui-sys-color-surface-container-lowest:var(--cm-sys-color-surface-elevation,#fff);--sdui-sys-color-surface-container-low:var(--cm-sys-color-surface-modal,#fff);--sdui-sys-color-surface-container:var(--cm-sys-color-surface-variant,#fafafa);--sdui-sys-color-surface-container-high:var(--cm-sys-color-container,#fafafa);--sdui-sys-color-surface-container-highest:var(--cm-sys-color-placeholder,#e8eaed);--sdui-sys-color-scrim:var(--cm-sys-color-scrim-modal,rgba(0,0,0,.32));--sdui-sys-color-shadow:var(--cm-sys-color-shadow,#000);--sdui-sys-color-outline:var(--cm-sys-color-outline,#80868b);--sdui-sys-color-outline-variant:var(--cm-sys-color-hairline,rgba(0,0,0,.12));--sdui-sys-color-disabled:var(--cm-sys-color-state-disabled,rgba(0,0,0,.54));--sdui-sys-color-disabled-container:var(--cm-sys-color-state-disabled-container,rgba(0,0,0,.04));--sdui-sys-color-focus-primary:var(--cm-sys-color-state-on-primary-on-surface-state,#1c3aa9);--sdui-sys-color-focus-primary-container:var(--cm-sys-color-state-primary-on-surface-focus,rgba(12,103,223,.12));--sdui-sys-color-focus-secondary:var(--cm-sys-color-state-on-neutral-on-surface-state,#202124);--sdui-sys-color-focus-secondary-container:var(--cm-sys-color-state-neutral-on-surface-focus,rgba(0,0,0,.08));--sdui-sys-color-hover-primary:var(--cm-sys-color-state-on-primary-on-surface-state,#1c3aa9);--sdui-sys-color-hover-primary-container:var(--cm-sys-color-state-primary-on-surface-hover,rgba(12,103,223,.04));--sdui-sys-color-hover-secondary:var(--cm-sys-color-state-on-neutral-on-surface-state,#202124);--sdui-sys-color-hover-secondary-container:var(--cm-sys-color-state-neutral-on-surface-hover,rgba(0,0,0,.04));--sdui-sys-color-pressed-primary:var(--cm-sys-color-state-on-primary-on-surface-state,#1c3aa9);--sdui-sys-color-pressed-primary-container:var(--cm-sys-color-state-primary-on-surface-active,rgba(12,103,223,.1));--sdui-sys-color-pressed-secondary:var(--cm-sys-color-state-on-neutral-on-surface-state,#202124);--sdui-sys-color-pressed-secondary-container:var(--cm-sys-color-state-neutral-on-surface-active,rgba(60,64,67,.1));--sdui-sys-color-status-success:var(--cm-sys-color-status-success,#0b8043);--sdui-sys-color-status-on-success:var(--cm-sys-color-status-on-success,#fff);--sdui-sys-color-status-success-container:var(--cm-sys-color-status-success-container,#e2f3eb);--sdui-sys-color-status-on-success-container:var(--cm-sys-color-on-surface,#000);--sdui-sys-color-status-warning:var(--cm-sys-color-status-warning,#dc6d00);--sdui-sys-color-status-on-warning:var(--cm-sys-color-status-on-warning,#fff);--sdui-sys-color-status-warning-container:var(--cm-sys-color-status-warning-container,#fef6e0);--sdui-sys-color-status-on-warning-container:var(--cm-sys-color-on-surface,#000);--sdui-sys-color-status-error:var(--cm-sys-color-status-error,#d50000);--sdui-sys-color-status-on-error:var(--cm-sys-color-status-on-error,#fff);--sdui-sys-color-status-error-container:var(--cm-sys-color-status-error-container,#fbe9e7);--sdui-sys-color-status-on-error-container:var(--cm-sys-color-on-surface,#000);--sdui-sys-color-status-neutral:var(--cm-sys-color-status-neutral,#80868b);--sdui-sys-color-status-on-neutral:var(--cm-sys-color-status-on-neutral,#000);--sdui-sys-color-status-neutral-container:var(--cm-sys-color-status-neutral-container,#fafafa);--sdui-sys-color-status-on-neutral-container:var(--cm-sys-color-on-surface,#000);--sdui-sys-color-link-default:var(--cm-sys-color-link-default,#3367d6);--sdui-sys-color-link-visited:var(--cm-sys-color-link-visited,#7b1fa2);--sdui-sys-color-charts-primary-outline:var(--cm-sys-color-charts-primary-outline,#0c67df);--sdui-sys-color-charts-primary-default:var(--cm-sys-color-charts-primary-default,#0c67df);--sdui-sys-color-charts-primary-medium:var(--cm-sys-color-charts-primary-medium,#8ab4f8);--sdui-sys-color-charts-primary-low:var(--cm-sys-color-charts-primary-low,#d2e3fc);--sdui-sys-color-charts-neutral-outline:var(--cm-sys-color-charts-neutral-outline,#80868b);--sdui-sys-color-charts-neutral-default:var(--cm-sys-color-charts-neutral-default,#80868b);--sdui-sys-color-charts-neutral-medium:var(--cm-sys-color-charts-neutral-medium,#dadce0);--sdui-sys-color-charts-neutral-low:var(--cm-sys-color-charts-neutral-low,#f1f3f4);--sdui-sys-color-charts-success-outline:var(--cm-sys-color-charts-success-outline,#188038);--sdui-sys-color-charts-success-default:var(--cm-sys-color-charts-success-default,#188038);--sdui-sys-color-charts-success-medium:var(--cm-sys-color-charts-success-medium,#81c995);--sdui-sys-color-charts-success-low:var(--cm-sys-color-charts-success-low,#ceead6);--sdui-sys-color-charts-warning-outline:var(--cm-sys-color-charts-warning-outline,#d93025);--sdui-sys-color-charts-warning-default:var(--cm-sys-color-charts-warning-default,#f9ab00);--sdui-sys-color-charts-warning-medium:var(--cm-sys-color-charts-warning-medium,#fdd663);--sdui-sys-color-charts-warning-low:var(--cm-sys-color-charts-warning-low,#feefc3);--sdui-sys-color-charts-error-outline:var(--cm-sys-color-charts-error-outline,#d93025);--sdui-sys-color-charts-error-default:var(--cm-sys-color-charts-error-default,#d93025);--sdui-sys-color-charts-error-medium:var(--cm-sys-color-charts-error-medium,#f28b82);--sdui-sys-color-charts-error-low:var(--cm-sys-color-charts-error-low,#fad2cf);--sdui-sys-color-charts-category1-outline:var(--cm-sys-color-charts-category1-outline,var(--cm-sys-color-charts-primary-outline));--sdui-sys-color-charts-category1-default:var(--cm-sys-color-charts-category1-default,var(--cm-sys-color-charts-primary-default));--sdui-sys-color-charts-category1-high:var(--cm-sys-color-charts-category1-high,#185abc);--sdui-sys-color-charts-category1-medium:var(--cm-sys-color-charts-category1-medium,var(--cm-sys-color-charts-primary-medium));--sdui-sys-color-charts-category1-low:var(--cm-sys-color-charts-category1-low,var(--cm-sys-color-charts-primary-low));--sdui-sys-color-charts-category2-outline:var(--cm-sys-color-charts-category2-outline,#e52592);--sdui-sys-color-charts-category2-default:var(--cm-sys-color-charts-category2-default,#e52592);--sdui-sys-color-charts-category2-high:var(--cm-sys-color-charts-category2-high,#b80672);--sdui-sys-color-charts-category2-medium:var(--cm-sys-color-charts-category2-medium,#ff8bcb);--sdui-sys-color-charts-category2-low:var(--cm-sys-color-charts-category2-low,#fdcfe8);--sdui-sys-color-charts-category3-outline:var(--cm-sys-color-charts-category3-outline,#12a4af);--sdui-sys-color-charts-category3-default:var(--cm-sys-color-charts-category3-default,#12b5cb);--sdui-sys-color-charts-category3-high:var(--cm-sys-color-charts-category3-high,#098591);--sdui-sys-color-charts-category3-medium:var(--cm-sys-color-charts-category3-medium,#78d9ec);--sdui-sys-color-charts-category3-low:var(--cm-sys-color-charts-category3-low,#cbf0f8);--sdui-sys-color-charts-category4-outline:var(--cm-sys-color-charts-category4-outline,#e8710a);--sdui-sys-color-charts-category4-default:var(--cm-sys-color-charts-category4-default,#e8710a);--sdui-sys-color-charts-category4-high:var(--cm-sys-color-charts-category4-high,#c26401);--sdui-sys-color-charts-category4-medium:var(--cm-sys-color-charts-category4-medium,#fcad70);--sdui-sys-color-charts-category4-low:var(--cm-sys-color-charts-category4-low,#fedfc8);--sdui-sys-color-charts-category5-outline:var(--cm-sys-color-charts-category5-outline,#9334e6);--sdui-sys-color-charts-category5-default:var(--cm-sys-color-charts-category5-default,#9334e6);--sdui-sys-color-charts-category5-high:var(--cm-sys-color-charts-category5-high,#7627bb);--sdui-sys-color-charts-category5-medium:var(--cm-sys-color-charts-category5-medium,#c58af9);--sdui-sys-color-charts-category5-low:var(--cm-sys-color-charts-category5-low,#e9d2fd);--sdui-sys-color-charts-category6-outline:var(--cm-sys-color-charts-category6-outline,#d93025);--sdui-sys-color-charts-category6-default:var(--cm-sys-color-charts-category6-default,#f9ab00);--sdui-sys-color-charts-category6-high:var(--cm-sys-color-charts-category6-high,#ea8600);--sdui-sys-color-charts-category6-medium:var(--cm-sys-color-charts-category6-medium,#fdd663);--sdui-sys-color-charts-category6-low:var(--cm-sys-color-charts-category6-low,#feefc3);--sdui-sys-color-charts-category7-outline:var(--cm-sys-color-charts-category7-outline,#188038);--sdui-sys-color-charts-category7-default:var(--cm-sys-color-charts-category7-default,#689f38);--sdui-sys-color-charts-category7-high:var(--cm-sys-color-charts-category7-high,#558b2f);--sdui-sys-color-charts-category7-medium:var(--cm-sys-color-charts-category7-medium,#aed581);--sdui-sys-color-charts-category7-low:var(--cm-sys-color-charts-category7-low,#dcedc8);--sdui-sys-color-charts-category8-outline:var(--cm-sys-color-charts-category8-outline,var(--cm-sys-color-charts-neutral-outline));--sdui-sys-color-charts-category8-default:var(--cm-sys-color-charts-category8-default,var(--cm-sys-color-charts-neutral-default));--sdui-sys-color-charts-category8-high:var(--cm-sys-color-charts-category8-high,#3c4043);--sdui-sys-color-charts-category8-medium:var(--cm-sys-color-charts-category8-medium,var(--cm-sys-color-charts-neutral-medium));--sdui-sys-color-charts-category8-low:var(--cm-sys-color-charts-category8-low,var(--cm-sys-color-charts-neutral-low));--sdui-sys-color-charts-area-category1-outline:var(--cm-sys-color-charts-area-category1-outline,#1967d2);--sdui-sys-color-charts-area-category1-fill:var(--cm-sys-color-charts-area-category1-fill,var(--cm-sys-color-charts-primary-low));--sdui-sys-color-charts-area-category2-outline:var(--cm-sys-color-charts-area-category2-outline,#c92786);--sdui-sys-color-charts-area-category2-fill:var(--cm-sys-color-charts-area-category2-fill,var(--cm-sys-color-charts-category2-low));--sdui-sys-color-charts-area-category3-outline:var(--cm-sys-color-charts-area-category3-outline,#007b83);--sdui-sys-color-charts-area-category3-fill:var(--cm-sys-color-charts-area-category3-fill,var(--cm-sys-color-charts-category3-low));--sdui-sys-color-charts-area-category4-outline:var(--cm-sys-color-charts-area-category4-outline,#e64a19);--sdui-sys-color-charts-area-category4-fill:var(--cm-sys-color-charts-area-category4-fill,#ffccbc);--sdui-sys-color-charts-area-category5-outline:var(--cm-sys-color-charts-area-category5-outline,#8430ce);--sdui-sys-color-charts-area-category5-fill:var(--cm-sys-color-charts-area-category5-fill,var(--cm-sys-color-charts-category5-low));--sdui-sys-color-charts-area-category6-outline:var(--cm-sys-color-charts-area-category6-outline,#c5221f);--sdui-sys-color-charts-area-category6-fill:var(--cm-sys-color-charts-area-category6-fill,var(--cm-sys-color-charts-category6-low));--sdui-sys-color-charts-area-category7-outline:var(--cm-sys-color-charts-area-category7-outline,#188038);--sdui-sys-color-charts-area-category7-fill:var(--cm-sys-color-charts-area-category7-fill,#ceead6);--sdui-sys-color-charts-area-category8-outline:var(--cm-sys-color-charts-area-category8-outline,#5f6368);--sdui-sys-color-charts-area-category8-fill:var(--cm-sys-color-charts-area-category8-fill,var(--cm-sys-color-charts-neutral-low));--sdui-sys-color-charts-line-category1:var(--cm-sys-color-charts-line-category1,var(--cm-sys-color-charts-primary-default));--sdui-sys-color-charts-line-category2:var(--cm-sys-color-charts-line-category2,#f439a0);--sdui-sys-color-charts-line-category3:var(--cm-sys-color-charts-line-category3,#12a4af);--sdui-sys-color-charts-line-category4:var(--cm-sys-color-charts-line-category4,#ff5722);--sdui-sys-color-charts-line-category5:var(--cm-sys-color-charts-line-category5,#af5cf7);--sdui-sys-color-charts-line-category6:var(--cm-sys-color-charts-line-category6,#689f38);--sdui-sys-color-charts-line-category7:var(--cm-sys-color-charts-line-category7,#3949ab);--sdui-sys-color-charts-line-category8:var(--cm-sys-color-charts-line-category8,var(--cm-sys-color-charts-neutral-default));--sdui-sys-color-charts-line-category9:var(--cm-sys-color-charts-line-category9,#188038);--sdui-sys-color-charts-line-category10:var(--cm-sys-color-charts-line-category10,#9c27b0);--sdui-sys-color-charts-line-category11:var(--cm-sys-color-charts-line-category11,#ea4335);--sdui-sys-color-charts-line-category12:var(--cm-sys-color-charts-line-category12,#039be5);--sdui-sys-color-charts-axis:var(--cm-sys-color-charts-axis,#80868b);--sdui-sys-color-charts-axis-label:var(--cm-sys-color-charts-axis-label,#5f6368);--sdui-sys-color-charts-legend-label:var(--cm-sys-color-charts-legend-label,#5f6368);--sdui-sys-color-charts-ticks:var(--cm-sys-color-charts-ticks,#80868b);--sdui-sys-color-charts-grid:var(--cm-sys-color-charts-grid,#dadce0);--sdui-sys-color-charts-threshold:var(--cm-sys-color-charts-threshold,#5f6368);--sdui-sys-color-charts-unfilled:var(--cm-sys-color-charts-unfilled,#e0e0e0);--sdui-sys-color-charts-card-surface:var(--cm-sys-color-charts-card-surface,var(--cm-sys-color-surface-modal));--sdui-sys-color-charts-title:var(--cm-sys-color-charts-title,#3c4043);--sdui-sys-color-charts-subtitle:var(--cm-sys-color-charts-subtitle,#5f6368);--sdui-ref-space-0:0;--sdui-ref-space-05:2px;--sdui-ref-space-1:4px;--sdui-ref-space-2:8px;--sdui-ref-space-3:12px;--sdui-ref-space-4:16px;--sdui-ref-space-5:20px;--sdui-ref-space-6:24px;--sdui-ref-space-7:28px;--sdui-ref-space-8:32px;--sdui-ref-space-9:36px;--sdui-ref-space-10:40px;--sdui-ref-space-12:48px;--sdui-ref-space-14:56px;--sdui-sys-type-display-small:var(--cm-sys-type-display-small,400 36px/44px \"Roboto\");--sdui-sys-type-display-medium:var(--cm-sys-type-display-medium,400 44px/52px \"Roboto\");--sdui-sys-type-display-large:var(--cm-sys-type-display-large,400 56px/64px \"Roboto\");--sdui-sys-type-headline-small:var(--cm-sys-type-headline-small,400 24px/32px \"Roboto\");--sdui-sys-type-headline-medium:var(--cm-sys-type-headline-medium,400 28px/36px \"Roboto\");--sdui-sys-type-headline-large:var(--cm-sys-type-headline-large,400 32px/40px \"Roboto\");--sdui-sys-type-title-small:var(--cm-sys-type-title-small,400 18px/24px \"Roboto\");--sdui-sys-type-title-medium:var(--cm-sys-type-title-medium,500 18px/24px \"Roboto\");--sdui-sys-type-title-large:var(--cm-sys-type-title-large,400 20px/28px \"Roboto\");--sdui-sys-type-label-small:var(--cm-sys-type-label-small,500 12px/16px \"Roboto\",sans-serif);--sdui-sys-type-label-medium:var(--cm-sys-type-label-medium,500 13px/20px \"Roboto\",sans-serif);--sdui-sys-type-label-large:var(--cm-sys-type-label-large,500 15px/20px \"Roboto\");--sdui-sys-type-body-small:var(--cm-sys-type-body-small,400 12px/16px \"Roboto\",sans-serif);--sdui-sys-type-body-medium:var(--cm-sys-type-body-medium,400 13px/20px \"Roboto\",sans-serif);--sdui-sys-type-body-large:var(--cm-sys-type-body-large,400 15px/20px \"Roboto\",sans-serif);--sdui-sys-type-code:var(--cm-sys-type-code,400 12px/16px \"Roboto Mono\",monospace);--sdui-ref-type-family-display:Google Sans,sans-serif;--sdui-ref-type-family-body:Roboto,sans-serif;--sdui-ref-type-family-code:Roboto Mono,monospace;--sdui-sys-color-charts-category1-outline:#8ab4f8;--sdui-sys-color-charts-category1-default:#8ab4f8;--sdui-sys-color-charts-category1-medium:#4285f4;--sdui-sys-color-charts-category1-low:#333e50;--sdui-sys-color-charts-category8-outline:#dadce0;--sdui-sys-color-charts-category8-default:#dadce0;--sdui-sys-color-charts-category8-medium:#9aa0a6;--sdui-sys-color-charts-category8-low:#47484a;--sdui-sys-color-charts-area-category1-fill:#333e50;--sdui-sys-color-charts-area-category2-fill:#4f3445;--sdui-sys-color-charts-area-category3-fill:#2f474d;--sdui-sys-color-charts-area-category5-fill:#423450;--sdui-sys-color-charts-area-category6-fill:#4f462c;--sdui-sys-color-charts-area-category8-fill:#4f484a;--sdui-sys-color-charts-line-category1:#8ab4f8;--sdui-sys-color-charts-line-category8:#fdd663;--sdui-sys-color-charts-card-surface:#2a2b2e;--sdui-sys-type-title-small:500 14px/21px \"Inter\",sans-serif;--sdui-sys-type-title-large:600 16px/24px \"Inter Tight\",sans-serif;--sdui-button-border-radius:12px;--sdui-button-font:500 14px/20px \"Inter\",sans-serif;--sdui-button-min-height:32px;--sdui-button-outline-color-focus:var(--color-v3-outline);--sdui-button-outline-offset-focus:-2px;--sdui-button-lowest-background-color-focus:var(--color-v3-button-container-high);--sdui-button-lowest-background-color-hover:var(--color-v3-button-container-high);--sdui-button-lowest-background-color:var(--color-v3-button-container);--sdui-button-lowest-border-color:var(--color-v3-outline);--sdui-button-lowest-border-width:1px;--sdui-button-lowest-box-shadow:var(--v3-shadow-xs);--sdui-button-lowest-color:var(--color-v3-text-on-button);--sdui-button-lowest-color-active:var(--sdui-button-low-color);--sdui-button-lowest-color-focus:var(--sdui-button-low-color);--sdui-button-lowest-color-hover:var(--sdui-button-low-color);--sdui-button-lowest-padding-left:12px;--sdui-button-lowest-padding-right:12px;--sdui-button-low-background-color-focus:var(--color-v3-button-container-high);--sdui-button-low-background-color-hover:var(--color-v3-button-container-high);--sdui-button-low-background-color:var(--color-v3-button-container);--sdui-button-low-border-color:var(--color-v3-outline);--sdui-button-low-border-width:1px;--sdui-button-low-box-shadow:var(--v3-shadow-xs);--sdui-button-low-color:var(--color-v3-text-on-button);--sdui-button-low-color-active:var(--sdui-button-low-color);--sdui-button-low-color-focus:var(--sdui-button-low-color);--sdui-button-low-color-hover:var(--sdui-button-low-color);--sdui-button-low-padding-left:12px;--sdui-button-low-padding-right:12px;--sdui-button-medium-background-color-focus:var(--color-v3-button-container-high);--sdui-button-medium-background-color-hover:var(--color-v3-button-container-high);--sdui-button-medium-background-color:var(--color-v3-button-container);--sdui-button-medium-border-color:var(--color-v3-outline);--sdui-button-medium-border-width:1px;--sdui-button-medium-box-shadow:var(--v3-shadow-xs);--sdui-button-medium-color:var(--color-v3-text-on-button);--sdui-button-medium-color-active:var(--sdui-button-low-color);--sdui-button-medium-color-focus:var(--sdui-button-low-color);--sdui-button-medium-color-hover:var(--sdui-button-low-color);--sdui-button-medium-padding-left:12px;--sdui-button-medium-padding-right:12px;--sdui-button-high-background-color-focus:var(--color-v3-button-container-high);--sdui-button-high-background-color-hover:var(--color-v3-button-container-high);--sdui-button-high-background-color:var(--color-v3-button-container);--sdui-button-high-border-color:var(--color-v3-outline);--sdui-button-high-border-width:1px;--sdui-button-high-box-shadow:var(--v3-shadow-xs);--sdui-button-high-color:var(--color-v3-text-on-button);--sdui-button-high-color-active:var(--sdui-button-low-color);--sdui-button-high-color-focus:var(--sdui-button-low-color);--sdui-button-high-color-hover:var(--sdui-button-low-color);--sdui-button-high-padding-left:12px;--sdui-button-high-padding-right:12px;--sdui-button-highest-background-color-focus:var(--color-v3-button-container-high);--sdui-button-highest-background-color-hover:var(--color-v3-button-container-high);--sdui-button-highest-background-color:var(--color-v3-button-container);--sdui-button-highest-border-color:var(--color-v3-outline);--sdui-button-highest-border-width:1px;--sdui-button-highest-box-shadow:var(--v3-shadow-xs);--sdui-button-highest-color:var(--color-v3-text-on-button);--sdui-button-highest-color-active:var(--sdui-button-low-color);--sdui-button-highest-color-focus:var(--sdui-button-low-color);--sdui-button-highest-color-hover:var(--sdui-button-low-color);--sdui-button-highest-padding-left:12px;--sdui-button-highest-padding-right:12px;--sdui-button-outline-width-focus:2px}"]
		});
		_.tMd = {
			Da: OLd,
			ke: () => {
				var { endpoint: a, szb: b, Ro: c } = _.m(DLd);
				return new OLd(a, null, { Opa: [new _.PKa(b, c)] });
			}
		};
		uMd = [[[
			"",
			"additionalDisclaimer",
			""
		]]];
		gLd = (a, b, c, d) => ({
			XQb: a,
			uRb: b,
			eRb: c,
			inline: d
		});
		_.x6 = class {
			constructor() {
				this.projectId = _.V();
				this.billingAccountId = _.Li.required();
				this.timeRange = _.V(6);
				this.I$a = _.V(true);
				this.Y$a = _.V(false);
				this.fKa = _.V(false);
				this.xh = _.Li([]);
				this.C8 = _.V(false);
				this.A = _.m(_.S5);
				this.R = _.m(_.ZC);
				this.UTa = _.Ni(_.w6);
				this.CS = _.W(() => this.R.F.EVb());
				this.Ln = _.W(() => [...this.A.Ln()]);
				this.er = _.W(() => [...this.A.er()]);
				this.z1 = _.oKd({ w2a: true });
				this.ve = {
					YNa: 302644,
					Nra: 279377,
					ZNa: 279378,
					EQa: 258937
				};
				this.I = _.M();
				this.Cs = _.W(() => {
					var a;
					return (a = this.I()) != null ? a : this.timeRange();
				});
				this.F = {
					value: "all",
					label: "All projects"
				};
				this.nF = _.W(() => {
					var a = this.xh();
					return [this.F, ...a.map((b) => ({
						value: b.Ya(),
						label: b.getDisplayName() || b.Ya()
					}))];
				});
				this.H = _.M([this.F]);
				this.O9a = this.H.asReadonly();
				this.U = _.W(() => {
					if (!this.fKa()) return this.projectId();
					var a = this.O9a();
					if (a.length !== 0 && a[0].value !== "all") return a[0].value;
				});
				this.w9a = _.W(() => {
					var a = this.A;
					var b = {
						billingAccountId: this.billingAccountId(),
						timeRange: this.Cs(),
						projectId: this.U()
					};
					if (b.billingAccountId) {
						b = VLd(a, b), a = new GLd(SLd(a.U.A, {
							intent: "BILLING_OVERVIEW_AI_STUDIO",
							DOb: "BillingSduiService",
							payload: b
						}));
					} else {
						a = undefined;
					}
					return a;
				});
				_.Fk([this.projectId, this.nF], () => {
					var a = this.projectId();
					var b = this.nF();
					b = a ? b.find((c) => c.value === a) : undefined;
					this.H.set([b != null ? b : this.F]);
				});
			}
			ib() {
				ULd(this.A);
			}
			Bba(a) {
				this.A.Bba(a);
			}
			Soa(a) {
				this.I.set(a);
			}
			Xn(a) {
				this.H.set(a);
			}
		};
		_.x6.prototype.R9 = _.ba(211);
		_.x6.J = function(a) {
			return new (a || _.x6)();
		};
		_.x6.ka = _.u({
			type: _.x6,
			da: [["ms-billing-sdui-chart"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.UTa, _.w6, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			inputs: {
				projectId: [1, "projectId"],
				billingAccountId: [1, "billingAccountId"],
				timeRange: [1, "timeRange"],
				I$a: [1, "showModelSelector"],
				Y$a: [1, "showTimeRangeSelector"],
				fKa: [1, "showProjectSelector"],
				xh: [1, "projectOptions"],
				C8: [1, "inlineSelectors"]
			},
			features: [_.yi([
				_.ELd,
				_.R5,
				_.Q5,
				_.tMd
			])],
			fc: ["[additionalDisclaimer]"],
			ha: 4,
			ia: 1,
			la: () => [
				["selectorsContainer", ""],
				" Cost information may take up to 24 hours to update. ",
				[1, "sdui-viewer-container"],
				[
					3,
					"ngTemplateOutlet",
					"ngTemplateOutletContext"
				],
				[3, "renderData"],
				[1, "sdui-viewer-disclaimer"],
				[
					3,
					"selectedTimeRange",
					"showSelectorLabel",
					"ve",
					"timeRangeOptions"
				],
				[
					"data-test-id",
					"project-selector",
					3,
					"multiple",
					"showSelectorLabel",
					"options",
					"selected",
					"ve"
				],
				[
					"data-test-id",
					"model-selector",
					3,
					"multiple",
					"showSelectorLabel",
					"options",
					"selected",
					"ve",
					"sortDesc"
				],
				[
					3,
					"onTimeRangeChange",
					"selectedTimeRange",
					"showSelectorLabel",
					"ve",
					"timeRangeOptions"
				],
				[
					"data-test-id",
					"project-selector",
					3,
					"onSelectionChange",
					"multiple",
					"showSelectorLabel",
					"options",
					"selected",
					"ve"
				],
				[
					"data-test-id",
					"model-selector",
					3,
					"onSelectionChange",
					"multiple",
					"showSelectorLabel",
					"options",
					"selected",
					"ve",
					"sortDesc"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.Xh(uMd), _.F(0, "div", 2), _.B(1, hLd, 6, 8), _.H(), _.z(2, lLd, 4, 7, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					_.y(), _.C(b.w9a() ? 1 : -1);
				}
			},
			dependencies: [
				_.tz,
				_.nz,
				_.w6,
				_.eE,
				_.H5,
				_.Cz,
				_.Bz
			],
			styles: [".sdui-viewer-container[_ngcontent-%COMP%]   .sdui-selectors[_ngcontent-%COMP%]{gap:16px;margin:16px 0;-webkit-flex-wrap:nowrap;-ms-flex-wrap:nowrap;flex-wrap:nowrap}.sdui-viewer-container[_ngcontent-%COMP%]   .sdui-selectors[_ngcontent-%COMP%] > *[_ngcontent-%COMP%]{-webkit-flex-shrink:1;-ms-flex-negative:1;flex-shrink:1;max-width:none;min-width:0;-webkit-flex-basis:auto;-ms-flex-preferred-size:auto;flex-basis:auto}@media screen and (max-width:480px){.sdui-viewer-container[_ngcontent-%COMP%]   .sdui-selectors[_ngcontent-%COMP%]{gap:8px}}.disclaimer[_ngcontent-%COMP%], .sdui-viewer-disclaimer[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var)}.sdui-viewer-container[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container);border:1px solid var(--color-v3-outline-var);border-radius:12px;box-shadow:var(--v3-shadow-xs);padding:16px}[_nghost-%COMP%]{display:block}.sdui-viewer-container[_ngcontent-%COMP%]{position:relative}.sdui-viewer-container[_ngcontent-%COMP%]   .sdui-selectors[_ngcontent-%COMP%]{margin:4px 0 20px 0;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}.sdui-viewer-container[_ngcontent-%COMP%]   .sdui-selector-inline[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:8px;position:absolute;top:0;right:0;z-index:10;padding:16px}.disclaimer[_ngcontent-%COMP%], .sdui-viewer-disclaimer[_ngcontent-%COMP%]{margin-top:16px}\n/*# sourceMappingURL=billing_sdui_chart.css.map */"]
		});
		ONd = function(a, b, c = "") {
			var d;
			return ((d = _.Isa(a / b, "en-US", "1.0-2")) != null ? d : "") + c;
		};
		_.A6 = class {
			transform(a) {
				if (a === null || a === undefined) return "-";
				var b = a < 0 ? "-" : "";
				a = Math.abs(a);
				a = a < 1e3 ? ONd(a, 1) : a < 1e6 ? ONd(a, 1e3, "K") : a < 1e9 ? ONd(a, 1e6, "M") : ONd(a, 1e9, "B");
				return `${b}${a}`;
			}
		};
		_.A6.J = function(a) {
			return new (a || _.A6)();
		};
		_.A6.Wo = _.Xe({
			name: "shortNumberPipe",
			type: _.A6,
			wk: true
		});
		PNd = function(a) {
			if (a & 1) {
				_.R(0), _.Ei(1, "shortNumberPipe");
			}
			if (a & 2) {
				a = _.K(2), _.S(" ", _.Fi(1, 1, a.limit()), " ");
			}
		};
		QNd = function(a) {
			if (a & 1) {
				_.F(0, "span"), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(2), _.y(), _.U(a.fta);
			}
		};
		RNd = function(a) {
			if (a & 1) {
				_.F(0, "span"), _.R(1, "-"), _.H();
			}
		};
		SNd = function(a) {
			if (a & 1) {
				_.F(0, "span", 4), _.R(1), _.Ei(2, "shortNumberPipe"), _.B(3, PNd, 2, 3)(4, QNd, 2, 1, "span")(5, RNd, 2, 0, "span"), _.H();
			}
			if (a & 2) {
				a = _.K(), _.y(), _.S("", _.Fi(2, 2, a.progress()), " / "), _.y(2), _.C(a.limit() >= 0 ? 3 : a.limit() === a.ysb ? 4 : 5);
			}
		};
		TNd = function(a) {
			if (a & 1) {
				_.F(0, "span", 6), _.R(1), _.H();
			}
			if (a & 2) {
				a = _.K(), _.P("ok-text", a.status() === "ok")("warning-text", a.status() === "warning")("exceeded-text", a.status() === "exceeded"), _.y(), _.S(" ", a.statusMessage(), " ");
			}
		};
		_.B6 = class {
			constructor() {
				this.limit = _.Li.required();
				this.progress = _.Li.required();
				this.Vab = _.V();
				this.Wab = _.V();
				this.Uab = _.V();
				this.F7a = _.Li(new Map([
					["ok", 0],
					["warning", .75],
					["exceeded", 1]
				]));
				this.g$ = _.V(0);
				this.showValues = _.V(true);
				this.Iv = _.V(false);
				this.SF = _.V("below");
				this.SUa = _.V("60px");
				this.RUa = _.V("8px");
				this.ysb = -1;
				this.fta = "Unlimited";
				this.E7a = _.W(() => {
					var a = this.limit();
					if (a === 0) return 0;
					var b;
					return ((b = this.progress()) != null ? b : 0) / a;
				});
				this.c_a = _.W(() => {
					var a = this.limit();
					var b;
					var c = (b = this.progress()) != null ? b : 0;
					b = this.g$();
					if (a < 0) return c > 0 ? b : 0;
					if (a === 0) return c > a ? 1 : 0;
					a = this.E7a();
					if (a > 0) {
						a = Math.max(a, b);
					}
					return Math.min(a, 1);
				});
				this.status = _.W(() => {
					var a = this.F7a();
					var b = this.c_a();
					var c = "unknown";
					for (let [d, e] of a) a = d, b >= e && (c = a);
					return c;
				});
				this.statusMessage = _.W(() => {
					var a = this.status();
					return a === "ok" ? this.Vab() : a === "warning" ? this.Wab() : a === "exceeded" ? this.Uab() : "";
				});
			}
		};
		_.B6.J = function(a) {
			return new (a || _.B6)();
		};
		_.B6.ka = _.u({
			type: _.B6,
			da: [["ms-status-bar"]],
			inputs: {
				limit: [1, "limit"],
				progress: [1, "progress"],
				Vab: [1, "statusOkMessage"],
				Wab: [1, "statusWarningMessage"],
				Uab: [1, "statusErrorMessage"],
				F7a: [1, "progressStatusMap"],
				g$: [1, "minimumProgressFraction"],
				showValues: [1, "showValues"],
				Iv: [1, "showTooltip"],
				SF: [1, "tooltipPosition"],
				SUa: [1, "barWidth"],
				RUa: [1, "barHeight"]
			},
			ha: 7,
			ia: 19,
			la: [
				[1, "status-bar"],
				[1, "bar-container"],
				[
					1,
					"fill-bar-container",
					3,
					"matTooltip",
					"matTooltipDisabled",
					"matTooltipPosition"
				],
				[1, "fill-bar"],
				[1, "metric-value"],
				[
					1,
					"status-message",
					3,
					"ok-text",
					"warning-text",
					"exceeded-text"
				],
				[1, "status-message"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 0)(1, "div", 1)(2, "div", 2), _.Ei(3, "percent"), _.I(4, "div", 3), _.H(), _.B(5, SNd, 6, 4, "span", 4), _.H(), _.B(6, TNd, 2, 7, "span", 5), _.H();
				}
				if (a & 2) {
					_.y(2), _.pi("height", b.RUa())("width", b.SUa()), _.E("matTooltip", b.Iv() ? _.Fi(3, 17, b.E7a()) : "")("matTooltipDisabled", !b.Iv())("matTooltipPosition", b.SF()), _.y(2), _.pi("width", b.c_a() * 100, "%"), _.P("ok", b.status() === "ok")("warning", b.status() === "warning")("exceeded", b.status() === "exceeded"), _.y(), _.C(b.showValues() ? 5 : -1), _.y(), _.C(b.statusMessage() ? 6 : -1);
				}
			},
			dependencies: [
				_.tz,
				_.IC,
				_.HC,
				_.rz,
				_.A6
			],
			styles: [".status-bar[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;gap:8px}.bar-container[_ngcontent-%COMP%]{-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex}.fill-bar-container[_ngcontent-%COMP%]{border-radius:12px;background-color:var(--color-v3-outline-var);margin-right:8px;overflow:hidden}.fill-bar-container[_ngcontent-%COMP%]   .fill-bar[_ngcontent-%COMP%]{border-radius:inherit;height:inherit}.ok[_ngcontent-%COMP%]{background-color:var(--color-v3-text-var)}.warning[_ngcontent-%COMP%]{background-color:var(--color-v3-accent-1)}.exceeded[_ngcontent-%COMP%]{background-color:var(--color-v3-accent-3)}.ok-text[_ngcontent-%COMP%]{color:var(--color-v3-text-var)}.warning-text[_ngcontent-%COMP%]{color:var(--color-v3-accent-1)}.exceeded-text[_ngcontent-%COMP%]{color:var(--color-v3-accent-3)}.metric-value[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;-webkit-font-feature-settings:\"tnum\";-moz-font-feature-settings:\"tnum\";font-feature-settings:\"tnum\";font-variant-numeric:tabular-nums;white-space:nowrap}"]
		});
		_.hr("yUkhDc");
		_.w6.prototype.R9 = _.ca(212, function() {
			{
				var a = this.veLoggingService;
				var b = new _.jz(301769);
				if (a.A) {
					a.A.na("click", b, undefined);
				}
			}
		});
		_.x6.prototype.R9 = _.ca(211, function() {
			var a;
			if (!((a = this.UTa()) == null)) {
				a.R9();
			}
		});
		var nOd = function(a) {
			_.x(function* () {
				var b = a.project();
				if (b) {
					var c = false;
					try {
						let [d, e] = yield Promise.all([_.Jjd(a.X, [12], 6, b, 1, [b.ah()]), _.Jjd(a.X, [13], 6, b, 1, [b.ah()])]);
						let f = (_.j3(d) || []).reduce((k, p) => k + _.Ajd(p), 0);
						let g = (_.j3(e) || []).reduce((k, p) => k + _.Ajd(p), 0);
						c = f > 1500 || g > 1500;
					} catch (d) {
						c = false;
					}
					if (a.project() === b) {
						a.H.set(c);
					}
				}
			});
		};
		var oOd = class {
			constructor() {
				this.S = _.Dk;
				this.ve = {
					Khb: 322003,
					Lhb: 322005,
					Mhb: 322004,
					akb: 309169,
					arb: 309170
				};
				this.dialog = _.m(_.rC);
				this.R = _.m(_.GG);
				this.U = _.m(_.Op);
				this.fa = _.m(_.EG);
				this.ma = _.m(_.ZC);
				this.A = _.m(_.T_);
				this.X = _.m(_.k3);
				this.H = _.M(false);
				this.F = _.W(() => this.U.getFlag(_.$Fb));
				this.aQb = _.W(() => !this.Sa() && !this.wL() && this.F() && this.k4a() !== -1 && this.H());
				this.hv = this.ma.A.small;
				this.uob = "project-usage-amount";
				this.vob = "project-usage-limit-card";
				this.project = _.W(() => this.fa.A());
				this.I = _.W(() => {
					var a;
					var b;
					return (a = this.project()) == null ? undefined : (b = _.au(a)) == null ? undefined : b.gk();
				});
				this.ea = _.W(() => {
					var a;
					return this.R.U().get((a = this.I()) != null ? a : "");
				});
				this.bBa = _.W(() => !!this.ea());
				this.oa = this.U.getFlag(_.VFb);
				this.disableReason = _.W(() => {
					if (!this.oa) return null;
					var a;
					if ((a = this.project()) == null ? 0 : _.Pm(a, 13)) return "Spend cap isn't available for this project because it's linked to first party's billing account.";
					var b;
					var c;
					a = (b = this.project()) == null ? undefined : (c = _.au(b)) == null ? undefined : _.Lm(c, 4);
					return a === 3 ? "Spend cap isn't available for this project because it's linked to an Invoiced (or Offline) billing account." : a === 5 ? "Spend cap isn't available for this project because it's linked to an internal billing account." : null;
				});
				this.permissions = _.W(() => {
					var a = this.project();
					return a ? new Set(a.Ap()) : new Set();
				});
				this.Bc = _.W(() => this.permissions().has(17));
				_.W(() => this.permissions().has(18));
				this.na = _.W(() => {
					var a = this.project();
					return (a ? a.Ap().includes(16) : false) && !this.A.R();
				});
				this.Sa = this.A.Sa;
				this.Pn = this.A.Pn;
				this.Wla = _.W(() => {
					var a;
					return (a = this.Pn()) == null ? undefined : _.S_(a);
				});
				this.C0a = _.W(() => this.Wla() !== undefined);
				this.qda = _.W(() => {
					var a;
					return (a = this.Pn()) == null ? undefined : _.Z(a, _.R_, 3);
				});
				this.aa = _.W(() => {
					var a;
					return (a = this.Pn()) == null ? undefined : _.Z(a, _.R_, 4);
				});
				this.k4a = _.W(() => {
					var a;
					return (a = _.t0(this.Wla())) != null ? a : -1;
				});
				this.zUb = _.W(() => {
					var a;
					return this.qda() ? (a = _.t0(this.qda(), false)) != null ? a : 0 : 0;
				});
				this.sQb = _.W(() => {
					var a = this.qda();
					return a && _.t0(a, false) ? _.t0(a) === 0 : false;
				});
				_.W(() => this.aa() ? _.t0(this.aa()) : null);
				this.AAb = "Current project spend";
				this.zAb = "Note: Updates may take up to 10 minutes. You may exceed your cap during this delay.";
				this.bSb = "You’re approaching the spend cap amount.";
				this.yKa = "You’ve reached the spend cap. Update it to unblock usage.";
				this.iLb = "You don't have permission to edit the spend cap.";
				this.aSb = "Overages may occur during 10 minutes latency. Resets on the first day of each month (PST).";
				this.zKa = "/gemini-api/docs/billing#project-spend-caps";
				this.Fxb = "/gemini-api/docs/billing#tier-spend-caps";
				this.wL = _.W(() => {
					var a = this.disableReason();
					return a !== null ? a : this.na() ? "" : "Permission denied. Please contact your project administrator for assistance.";
				});
				this.hHb = _.W(() => {
					var a;
					var b;
					return (b = (a = this.Pn()) == null ? undefined : _.Pm(a, 5)) != null ? b : false;
				});
				this.Ixb = ["/billing"];
				this.Hxb = _.W(() => {
					var a = this.I();
					return (a = a ? _.Pn(a) : "") ? { billing: a } : {};
				});
				_.Fk([this.project], () => {
					var a;
					var b = this.A;
					var c = (a = this.project()) != null ? a : undefined;
					b.project.set(c);
				});
				_.Fk([
					this.F,
					this.project,
					this.wL,
					this.Sa
				], () => {
					if (this.F() && !this.Sa() && !this.wL()) {
						var a = this.project();
						if (a && a.ah() !== 20) {
							nOd(this);
						} else {
							this.H.set(false);
						}
					}
				});
			}
			ib() {
				_.FG(this.R).catch(() => {});
			}
		};
		oOd.J = function(a) {
			return new (a || oOd)();
		};
		oOd.ka = _.u({
			type: oOd,
			da: [["ms-project-usage-limit"]],
			ha: 15,
			ia: 6,
			la: () => [
				["dataRefreshTooltip", ""],
				"Monthly spend cap",
				" You've reached the �#4��*5:1��#1:1�billing account�/#1:1��/*5:1��*6:2��#1:2�billing account�/#1:2��/*6:2��/#4� monthly spend cap and service has been paused. ",
				"Learn more",
				" Spend Caps currently do not support costs related to search and maps grounding. ",
				"Experimental",
				"Edit spend cap",
				"Set spend cap",
				" Learn more ",
				[
					"calloutType",
					"error",
					1,
					"callout",
					3,
					"isDismissable",
					"ve",
					"veImpression"
				],
				[
					"calloutType",
					"warning",
					1,
					"callout",
					3,
					"isDismissable"
				],
				[1, "container"],
				[1, "title-container"],
				[1, "title"],
				[1, "experimental-label"],
				[
					"ms-button",
					"",
					"data-test-id",
					"edit-budget-button",
					"aria-label",
					"Set or edit spend cap",
					3,
					"variant",
					"iconName",
					"disabled",
					"matTooltipDisabled",
					"matTooltip",
					"ve",
					"veClick",
					"veImpression",
					"xapTourElementId"
				],
				[1, "loading-container"],
				[1, "error-state-message"],
				[1, "disclaimer"],
				[
					"callout-content",
					"",
					1,
					"callout-content"
				],
				[
					"data-test-id",
					"billing-account-link",
					3,
					"routerLink",
					"queryParams",
					"ve",
					"veClick",
					"veImpression"
				],
				["data-test-id", "billing-account-text"],
				[
					"data-test-id",
					"learn-more-link",
					3,
					"documentation-path",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					"ms-button",
					"",
					"data-test-id",
					"edit-budget-button",
					"aria-label",
					"Set or edit spend cap",
					3,
					"click",
					"variant",
					"iconName",
					"disabled",
					"matTooltipDisabled",
					"matTooltip",
					"ve",
					"veClick",
					"veImpression",
					"xapTourElementId"
				],
				["diameter", "24"],
				[1, "usage-limit-container"],
				[
					1,
					"usage",
					3,
					"xapTourElementId"
				],
				[1, "limit"],
				[1, "status-bar-container"],
				[
					3,
					"limit",
					"progress",
					"minimumProgressFraction",
					"showValues",
					"barWidth",
					"statusWarningMessage",
					"statusErrorMessage"
				],
				[
					1,
					"error-icon",
					3,
					"iconName"
				],
				[
					"data-test-id",
					"learn-more-link",
					3,
					"documentation-path"
				],
				[1, "data-refresh-tooltip"],
				[1, "data-refresh-tooltip-title"],
				[1, "data-refresh-tooltip-content"]
			],
			template: function(a, b) {
				if (a & 1) {
					_.B(0, WNd, 9, 8, "ms-callout", 9), _.B(1, XNd, 4, 1, "ms-callout", 10), _.F(2, "div", 11)(3, "div", 12)(4, "div")(5, "label", 13), _.Mh(6, 1), _.H(), _.B(7, YNd, 2, 0, "span", 14), _.H(), _.B(8, bOd, 2, 10, "button", 15), _.H(), _.B(9, cOd, 2, 0, "div", 16)(10, dOd, 10, 15), _.B(11, eOd, 4, 2, "div", 17)(12, fOd, 5, 2, "div", 18), _.H(), _.z(13, gOd, 5, 2, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					_.C(b.hHb() ? 0 : -1), _.y(), _.C(b.aQb() ? 1 : -1), _.y(6), _.C(b.wL() ? -1 : 7), _.y(), _.C(b.wL() ? -1 : 8), _.y(), _.C(b.Sa() ? 9 : b.wL() ? -1 : 10), _.y(2), _.C(b.wL() ? 11 : 12);
				}
			},
			dependencies: [
				_.Yy,
				_.zA,
				_.LC,
				_.dz,
				_.zC,
				_.yC,
				_.HC,
				_.sA,
				_.B6,
				_.Cz,
				_.Bz,
				_.P3,
				_.u0
			],
			styles: [".callout[_ngcontent-%COMP%]{margin-bottom:24px;max-width:670px}.container[_ngcontent-%COMP%]{background-color:var(--color-v3-surface-container);border:1px solid var(--color-v3-outline-var);border-radius:12px;box-shadow:var(--v3-shadow-xs);color:var(--color-v3-text-var);max-width:670px;padding:16px;width:100%}.title-container[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;-webkit-box-pack:justify;-webkit-justify-content:space-between;-moz-box-pack:justify;-ms-flex-pack:justify;justify-content:space-between;margin-bottom:12px}.title[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;color:var(--color-v3-text)}.experimental-label[_ngcontent-%COMP%]{border-radius:8px;padding:1px 6px 1px 5px;border:1px solid var(--color-v3-outline);background-color:var(--color-v3-surface-container-high);color:var(--color-v3-text);display:-webkit-inline-box;display:-webkit-inline-flex;display:-moz-inline-box;display:-ms-inline-flexbox;display:inline-flex;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;gap:5px;font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text-var);padding-left:6px;margin-left:4px}.experimental-label[_ngcontent-%COMP%]:before{content:\"\";width:6px;aspect-ratio:1/1;border-radius:50%;-webkit-flex-shrink:0;-ms-flex-negative:0;flex-shrink:0}.experimental-label.enabled[_ngcontent-%COMP%]:before, .experimental-label.green[_ngcontent-%COMP%]:before, .experimental-label.new[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-4)}.experimental-label.gray[_ngcontent-%COMP%]:before, .experimental-label.not-enabled[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-var)}.experimental-label.confidential[_ngcontent-%COMP%]:before, .experimental-label.orange[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-1)}.experimental-label.blue[_ngcontent-%COMP%]:before, .experimental-label.paid[_ngcontent-%COMP%]:before{background-color:var(--color-v3-text-link)}.experimental-label.alert[_ngcontent-%COMP%]:before, .experimental-label.red[_ngcontent-%COMP%]:before{background-color:var(--color-v3-accent-3)}.experimental-label.hide-circle[_ngcontent-%COMP%]:before{display:none}.experimental-label[_ngcontent-%COMP%]:before{display:none}.loading-container[_ngcontent-%COMP%]{margin:8px 0;padding-left:32px}.usage-limit-container[_ngcontent-%COMP%]{font-family:Inter Tight,sans-serif;font-optical-sizing:auto;font-size:24px;font-weight:600;line-height:32px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:4px;margin:8px 0}.usage[_ngcontent-%COMP%]{color:var(--color-v3-text)}.limit[_ngcontent-%COMP%]{color:var(--color-v3-text-var)}.status-bar-container[_ngcontent-%COMP%]{margin:8px auto}.usage-tooltip[_ngcontent-%COMP%]{color:var(--color-v3-text);padding:8px 12px}.usage-tooltip-money[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:500;line-height:21px;display:block}.usage-tooltip-label[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;display:block}.disclaimer[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var);margin-top:16px}.error-state-message[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:14px;font-weight:400;line-height:21px;-webkit-box-align:center;-webkit-align-items:center;-moz-box-align:center;-ms-flex-align:center;align-items:center;color:var(--color-v3-text);display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:8px}.error-icon[_ngcontent-%COMP%]{color:var(--color-v3-accent-3)}.data-refresh-tooltip[_ngcontent-%COMP%]{width:285px;padding:8px 12px}.data-refresh-tooltip-title[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:16px;color:var(--color-v3-text);display:block}.data-refresh-tooltip-content[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var);display:block}"]
		});
		var pOd = function(a) {
			_.x(function* () {
				try {
					yield _.Gy(a.Za);
				} catch (b) {
					_.Mw(a.U, Error("Ki"));
				}
			});
		};
		_.ls = class {
			constructor() {
				this.ZUa = _.Ni(_.x6);
				this.Ul = _.OG;
				this.ve = {
					Ohb: 265318,
					EQa: 258937,
					YNa: 302644,
					Nra: 279377,
					ZNa: 279378,
					Thb: 325154
				};
				this.aa = _.m(_.Qu);
				this.Hb = _.Nn(this.aa.Oe);
				this.Za = _.m(_.Iy);
				this.R = _.m(_.CG);
				this.U = _.m(_.Nw);
				this.I = _.m(_.Ou);
				this.X = _.m(_.Op);
				this.A = _.m(_.S5);
				this.F = _.m(_.EG);
				this.ma = _.m(_.ZC);
				this.dialog = _.m(_.rC);
				this.ea = _.m(_.O3);
				this.H = _.m(_.yG);
				this.window = _.m(_.Sn);
				this.fa = this.X.getFlag(_.iL);
				this.Q2 = "Gemini API Spend";
				this.dz = this.Za.dz;
				this.CS = _.W(() => this.ma.F.Ola());
				this.xa = this.F.A;
				this.Eo = this.Za.je;
				this.xh = this.Za.Sd;
				this.Xn = (a) => {
					this.F.Xn(a);
				};
				this.TE = {
					text: "Docs",
					click: () => {
						this.cF();
					},
					t1: "learn-more-button",
					tooltip: "Documentation for Gemini API spend cap",
					ve: this.ve.Thb
				};
				this.Ln = this.A.Ln;
				this.er = this.A.er;
				this.Bba = (a) => {
					this.A.Bba(a);
				};
				this.State = qOd;
				this.state = _.W(() => !this.xa() || this.dz() ? 0 : this.billingAccountId() ? 2 : 1);
				this.z1 = _.oKd({ w2a: true });
				this.Cs = _.M(3);
				this.projectName = _.W(() => {
					var a = this.xa();
					return a ? a.getDisplayName() || a.Ya() : "";
				});
				this.projectId = _.W(() => {
					var a = this.xa();
					if (a) return a.Ya();
				});
				this.billingAccountId = _.W(() => {
					var a;
					var b;
					var c = (a = this.xa()) == null ? undefined : (b = _.au(a)) == null ? undefined : b.gk();
					if (c) return c.split("/")[1];
				});
				_.Fk([this.state, this.H.A], () => {
					if (this.state() === 2 && this.H.A() && this.fa) {
						let a = _.I3();
						this.ea.Fi.next(a);
					}
				});
			}
			ib() {
				pOd(this);
			}
			cF() {
				_.rd(this.window, _.jd("https://ai.google.dev/gemini-api/docs/billing#spend-caps"), "_blank");
			}
			fC() {
				var a = this.xa();
				if (a) {
					_.Rn(this.I, "USAGE", "Clicked Set up billing button on a Project"), this.dialog.open(_.MG, {
						id: "oaas-dialog",
						data: { st: a }
					});
				}
			}
			T$() {
				var a = this.projectId();
				if (a) {
					var b;
					if (!((b = this.ZUa()) == null)) {
						b.R9();
					}
					_.AUc(this.R, a);
				}
			}
		};
		_.ls.J = function(a) {
			return new (a || _.ls)();
		};
		_.ls.ka = _.u({
			type: _.ls,
			da: [["ms-billing-dashboard"]],
			Ka: function(a, b) {
				if (a & 1) {
					_.ji(b.ZUa, _.x6, 5);
				}
				if (a & 2) {
					_.ki();
				}
			},
			features: [_.yi([
				_.ELd,
				_.R5,
				_.Q5,
				_.tMd,
				_.S5
			])],
			ha: 8,
			ia: 3,
			la: () => [
				["selectorsSection", ""],
				"There is no billing currently set up for this project ",
				" Set up billing ",
				" Billing total reflects only Gemini API usage and does not include usage of other Google Cloud products or Google AI Studio. ",
				" Open in Cloud Console ",
				[1, "page-content-wrapper"],
				[1, "page-content-inner-wrapper"],
				[
					3,
					"headerText",
					"learnMoreButton"
				],
				[3, "ngTemplateOutlet"],
				[
					"headline",
					"Import or create a project",
					"message",
					"Only projects you import from Google Cloud will appear on this page.",
					"learnMoreUrl",
					"https://ai.google.dev/gemini-api/docs/api-key#import-projects",
					3,
					"showSparkle",
					"showImportProjectsButton",
					"showCreateProjectButton"
				],
				[1, "no-billing-account"],
				[1, "project-name"],
				[
					"ms-button",
					"",
					"aria-label",
					"Set up billing",
					3,
					"click",
					"disabled",
					"xapInlineDialog",
					"ve",
					"veClick",
					"veImpression"
				],
				[
					3,
					"billingAccountId",
					"projectId",
					"timeRange",
					"showModelSelector",
					"showTimeRangeSelector"
				],
				[
					"data-test-id",
					"disclaimer",
					1,
					"disclaimer"
				],
				[
					"href",
					"https://console.cloud.google.com/billing",
					"id",
					"open-in-cloud-console",
					3,
					"click",
					"ve",
					"veClick"
				],
				[1, "selectors"],
				[
					3,
					"onProjectSelectionChange",
					"showImportProjectOption",
					"showSelectorLabel",
					"projectOptions",
					"selectedProject",
					"isLoading",
					"showProjectIds",
					"ve"
				],
				[
					"diameter",
					"36",
					1,
					"spinner-container"
				]
			],
			template: function(a, b) {
				if (a & 1) {
					_.F(0, "div", 5)(1, "div", 6), _.I(2, "ms-dashboard-header", 7), _.B(3, iOd, 2, 2)(4, jOd, 10, 7)(5, kOd, 10, 8), _.H()(), _.z(6, mOd, 3, 8, "ng-template", null, 0, _.Ii);
				}
				if (a & 2) {
					let c;
					_.y(2);
					_.E("headerText", b.Q2)("learnMoreButton", b.TE);
					_.y();
					_.C((c = b.state()) === b.State.Jnb ? 3 : c === b.State.Fnb ? 4 : c === b.State.nqb ? 5 : -1);
				}
			},
			dependencies: [
				_.x6,
				_.Yy,
				_.tz,
				_.nz,
				_.I5,
				_.zC,
				_.yC,
				_.xE,
				oOd,
				_.n3,
				_.Cz,
				_.Bz,
				_.EC
			],
			styles: ["section[_ngcontent-%COMP%]{margin:16px 0}.selectors[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;gap:16px;margin:16px 0;-webkit-flex-wrap:nowrap;-ms-flex-wrap:nowrap;flex-wrap:nowrap}.selectors[_ngcontent-%COMP%] > *[_ngcontent-%COMP%]{-webkit-flex-shrink:1;-ms-flex-negative:1;flex-shrink:1;max-width:none;min-width:0;-webkit-flex-basis:auto;-ms-flex-preferred-size:auto;flex-basis:auto}@media screen and (max-width:480px){.selectors[_ngcontent-%COMP%]{gap:8px}}.spinner-container[_ngcontent-%COMP%]{display:block;margin:4px auto;background-color:var(--color-v3-surface-container);width:-webkit-fit-content;width:-moz-fit-content;width:fit-content}.disclaimer[_ngcontent-%COMP%]{font-family:Inter,sans-serif;font-optical-sizing:auto;font-size:12px;font-weight:400;line-height:18px;color:var(--color-v3-text-var)}[_nghost-%COMP%]{--chart-axis-label-light:#5d5d5f;--chart-grid-light:#eeeeec;--chart-axis-label-dark:#8c8c8c;--chart-grid-dark:#2a2a2a}[_nghost-%COMP%]   .page-content-inner-wrapper[_ngcontent-%COMP%]{max-width:min(1400px,90%)}@media screen and (max-width:600px){[_nghost-%COMP%]   .page-content-inner-wrapper[_ngcontent-%COMP%]{max-width:100%}}section[_ngcontent-%COMP%]{margin:12px 0}.selectors[_ngcontent-%COMP%]{margin:4px 0 8px 0}.no-billing-account[_ngcontent-%COMP%]{display:-webkit-box;display:-webkit-flex;display:-moz-box;display:-ms-flexbox;display:flex;-webkit-box-orient:vertical;-webkit-box-direction:normal;-webkit-flex-direction:column;-moz-box-orient:vertical;-moz-box-direction:normal;-ms-flex-direction:column;flex-direction:column;-webkit-box-align:start;-webkit-align-items:flex-start;-moz-box-align:start;-ms-flex-align:start;align-items:flex-start;gap:20px;margin:20px 0}.disclaimer[_ngcontent-%COMP%]{margin-top:16px}"]
		});
		_.ir();
	} catch (e) {
		_._DumpException(e);
	}
}).call(this, this.default_MakerSuite);
// Google Inc.

