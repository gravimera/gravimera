import "./style.css";

import { fromBinary } from "@bufbuild/protobuf";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

import {
  SceneDatMeshKey,
  SceneDatSchema,
  SceneTerrainDatSchema,
  type ColorDat,
  type SceneDat,
  type SceneDatObjectDef,
  type SceneDatPartDef,
  type SceneDatPrimitive,
  type SceneDatPrimitiveMeshRef,
  type SceneDatPrimitiveSolid,
  type SceneDatTransform,
  type TerrainDefV1,
  type Uuid128,
} from "gravimera-proto";

type Rgba = { r: number; g: number; b: number; a: number };

function clamp01(x: number): number {
  if (!Number.isFinite(x)) return 0;
  return Math.max(0, Math.min(1, x));
}

function rgbaFromPackedLeU32(rgba: number): Rgba {
  const u = rgba >>> 0;
  const r = (u & 0xff) / 255;
  const g = ((u >>> 8) & 0xff) / 255;
  const b = ((u >>> 16) & 0xff) / 255;
  const a = ((u >>> 24) & 0xff) / 255;
  return { r, g, b, a };
}

function rgbaFromColorDat(dat?: ColorDat): Rgba {
  const packed = dat?.rgba ?? 0xffffffff;
  return rgbaFromPackedLeU32(packed);
}

function mulRgba(a: Rgba, b: Rgba): Rgba {
  return {
    r: clamp01(a.r * b.r),
    g: clamp01(a.g * b.g),
    b: clamp01(a.b * b.b),
    a: clamp01(a.a * b.a),
  };
}

function uuidToU128(uuid?: Uuid128): bigint | null {
  if (!uuid) return null;
  return (uuid.hi << 64n) | uuid.lo;
}

function transformToMatrix(t?: SceneDatTransform): THREE.Matrix4 {
  if (!t) return new THREE.Matrix4().identity();
  const pos = new THREE.Vector3(t.tx, t.ty, t.tz);
  const rot = new THREE.Quaternion(t.rx, t.ry, t.rz, t.rw);
  const scale = new THREE.Vector3(t.sx, t.sy, t.sz);
  return new THREE.Matrix4().compose(pos, rot, scale);
}

function setObjectLocalMatrix(obj: THREE.Object3D, m: THREE.Matrix4) {
  obj.matrixAutoUpdate = false;
  obj.matrix.copy(m);
}

function logLine(el: HTMLElement, line: string) {
  el.textContent = `${line}\n${el.textContent ?? ""}`.slice(0, 8000);
}

function mustGetEl<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
}

const canvas = mustGetEl<HTMLCanvasElement>("viewport");
const uiEl = mustGetEl<HTMLElement>("ui");
const logEl = mustGetEl<HTMLElement>("log");
const sceneFileEl = mustGetEl<HTMLInputElement>("sceneFile");
const terrainFileEl = mustGetEl<HTMLInputElement>("terrainFile");
const loadBtnEl = mustGetEl<HTMLButtonElement>("loadBtn");
const togglePanelBtnEl = mustGetEl<HTMLButtonElement>("togglePanelBtn");
const filePanelEl = mustGetEl<HTMLElement>("filePanel");

const moveFwdBtnEl = mustGetEl<HTMLButtonElement>("moveFwdBtn");
const moveLeftBtnEl = mustGetEl<HTMLButtonElement>("moveLeftBtn");
const moveBackBtnEl = mustGetEl<HTMLButtonElement>("moveBackBtn");
const moveRightBtnEl = mustGetEl<HTMLButtonElement>("moveRightBtn");

const PANEL_COLLAPSED_KEY = "gravimera.web_viewer.panel_collapsed";
const storedPanelCollapsed = localStorage.getItem(PANEL_COLLAPSED_KEY);
// Default: panel hidden (better for mobile). If the user previously toggled, honor the preference.
let panelCollapsed = storedPanelCollapsed === null ? true : storedPanelCollapsed === "1";

function applyPanelCollapsedState() {
  uiEl.classList.toggle("ui-collapsed", panelCollapsed);
  filePanelEl.hidden = panelCollapsed;
  togglePanelBtnEl.textContent = panelCollapsed ? "Show Panel" : "Hide Panel";
}

applyPanelCollapsedState();
togglePanelBtnEl.addEventListener("click", () => {
  panelCollapsed = !panelCollapsed;
  localStorage.setItem(PANEL_COLLAPSED_KEY, panelCollapsed ? "1" : "0");
  applyPanelCollapsedState();
});

const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));

const threeScene = new THREE.Scene();
threeScene.background = new THREE.Color(0xf4f7fb);

const camera = new THREE.PerspectiveCamera(55, 1, 0.01, 5000);
camera.position.set(8, 6, 10);

const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.dampingFactor = 0.08;
// Make orbit drag feel like "grab the scene and move it" (matches Gravimera's in-game camera feel).
controls.rotateSpeed = -1.0;
controls.target.set(0, 0.6, 0);
controls.update();

const clock = new THREE.Clock();

function isTextInputTarget(target: EventTarget | null): boolean {
  if (!target || !(target instanceof HTMLElement)) return false;
  const tag = target.tagName.toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") return true;
  return target.isContentEditable;
}

const cameraKeys = new Set<string>([
  "KeyW",
  "KeyA",
  "KeyS",
  "KeyD",
  "ArrowUp",
  "ArrowLeft",
  "ArrowDown",
  "ArrowRight",
  "KeyQ",
  "KeyE",
  "KeyZ",
  "KeyX",
  "ShiftLeft",
  "ShiftRight",
]);
const pressedKeys = new Set<string>();

function bindHoldToKey(btn: HTMLButtonElement, keyCode: string) {
  let activePointer: number | null = null;

  const release = (e?: PointerEvent) => {
    if (e && activePointer !== null && e.pointerId !== activePointer) return;
    if (activePointer !== null) {
      try {
        btn.releasePointerCapture(activePointer);
      } catch {
        // Ignore capture errors (can happen if capture was lost).
      }
    }
    activePointer = null;
    pressedKeys.delete(keyCode);
    btn.dataset.active = "0";
  };

  btn.addEventListener(
    "pointerdown",
    (e) => {
      // Only handle primary action.
      if (e.pointerType === "mouse" && e.button !== 0) return;
      activePointer = e.pointerId;
      btn.setPointerCapture(e.pointerId);
      pressedKeys.add(keyCode);
      btn.dataset.active = "1";
      e.preventDefault();
      e.stopPropagation();
    },
    { passive: false },
  );

  btn.addEventListener("pointerup", release);
  btn.addEventListener("pointercancel", release);
  btn.addEventListener("lostpointercapture", () => release());
  btn.addEventListener("contextmenu", (e) => e.preventDefault());
}

bindHoldToKey(moveFwdBtnEl, "KeyW");
bindHoldToKey(moveLeftBtnEl, "KeyA");
bindHoldToKey(moveBackBtnEl, "KeyS");
bindHoldToKey(moveRightBtnEl, "KeyD");

window.addEventListener(
  "keydown",
  (e) => {
    // Avoid breaking browser/app shortcuts and file inputs.
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (isTextInputTarget(e.target)) return;
    if (!cameraKeys.has(e.code)) return;
    pressedKeys.add(e.code);
    e.preventDefault();
  },
  { passive: false },
);

window.addEventListener(
  "keyup",
  (e) => {
    if (!cameraKeys.has(e.code)) return;
    pressedKeys.delete(e.code);
    e.preventDefault();
  },
  { passive: false },
);

window.addEventListener("blur", () => {
  pressedKeys.clear();
});

// Access OrbitControls' internal delta mutators so we can still call `controls.update()` only once per
// frame (public `rotateLeft/rotateUp/pan` call `update()` internally and would apply damping twice).
const controlsInternal = controls as unknown as {
  _rotateLeft: (angle: number) => void;
  _rotateUp: (angle: number) => void;
};

const worldUp = new THREE.Vector3(0, 1, 0);
const tmpForward = new THREE.Vector3();
const tmpRight = new THREE.Vector3();
const tmpDelta = new THREE.Vector3();

function applyCameraKeyControls(dtSeconds: number) {
  if (pressedKeys.size === 0) return;

  const dt = Math.min(0.05, Math.max(0, dtSeconds));
  if (dt <= 0) return;

  const boost = pressedKeys.has("ShiftLeft") || pressedKeys.has("ShiftRight") ? 2.5 : 1.0;

  const dist = camera.position.distanceTo(controls.target);
  const panSpeedUnitsPerSec = Math.min(80, Math.max(0.5, dist * 1.1)) * boost;
  const rotateSpeedRadPerSec = 1.25 * (boost > 1 ? 1.35 : 1.0);

  let moveRight = 0;
  let moveForward = 0;
  if (pressedKeys.has("KeyD") || pressedKeys.has("ArrowRight")) moveRight += 1;
  if (pressedKeys.has("KeyA") || pressedKeys.has("ArrowLeft")) moveRight -= 1;
  if (pressedKeys.has("KeyW") || pressedKeys.has("ArrowUp")) moveForward += 1;
  if (pressedKeys.has("KeyS") || pressedKeys.has("ArrowDown")) moveForward -= 1;

  if (moveRight !== 0 || moveForward !== 0) {
    camera.getWorldDirection(tmpForward);
    tmpForward.y = 0;
    if (tmpForward.lengthSq() < 1e-10) {
      tmpForward.set(0, 0, -1);
    }
    tmpForward.normalize();
    tmpRight.crossVectors(tmpForward, worldUp).normalize();

    tmpDelta.set(0, 0, 0);
    tmpDelta.addScaledVector(tmpRight, moveRight);
    tmpDelta.addScaledVector(tmpForward, moveForward);
    if (tmpDelta.lengthSq() > 1e-10) {
      tmpDelta.normalize().multiplyScalar(panSpeedUnitsPerSec * dt);
      camera.position.add(tmpDelta);
      controls.target.add(tmpDelta);
    }
  }

  const rot = rotateSpeedRadPerSec * dt;
  if (pressedKeys.has("KeyQ")) controlsInternal._rotateLeft(rot);
  if (pressedKeys.has("KeyE")) controlsInternal._rotateLeft(-rot);
  if (pressedKeys.has("KeyZ")) controlsInternal._rotateUp(rot);
  if (pressedKeys.has("KeyX")) controlsInternal._rotateUp(-rot);
}

threeScene.add(new THREE.AmbientLight(0xffffff, 0.72));
threeScene.add(new THREE.HemisphereLight(0xffffff, 0xcfd8e3, 0.22));
const dir = new THREE.DirectionalLight(0xffffff, 0.95);
dir.position.set(5, 10, 4);
threeScene.add(dir);

const worldRoot = new THREE.Group();
threeScene.add(worldRoot);

const grid = new THREE.GridHelper(50, 50, 0x95a6bc, 0xd9e0ea);
grid.position.y = 0.001;
worldRoot.add(grid);

const geometryCache = new Map<string, THREE.BufferGeometry>();

function getPrimitiveGeometry(
  mesh: SceneDatMeshKey,
  params: SceneDatPrimitiveSolid["params"] | undefined,
): THREE.BufferGeometry {
  const paramsKey = params ? JSON.stringify(params) : "";
  const key = `${mesh}|${paramsKey}`;
  const cached = geometryCache.get(key);
  if (cached) return cached;

  let geom: THREE.BufferGeometry;
  switch (mesh) {
    case SceneDatMeshKey.UNIT_CUBE:
      geom = new THREE.BoxGeometry(1, 1, 1);
      break;
    case SceneDatMeshKey.UNIT_CYLINDER:
      geom = new THREE.CylinderGeometry(0.5, 0.5, 1, 24, 1);
      break;
    case SceneDatMeshKey.UNIT_CONE:
      geom = new THREE.ConeGeometry(0.5, 1, 24, 1);
      break;
    case SceneDatMeshKey.UNIT_SPHERE:
      geom = new THREE.SphereGeometry(0.5, 24, 16);
      break;
    case SceneDatMeshKey.UNIT_PLANE: {
      const g = new THREE.PlaneGeometry(1, 1, 1, 1);
      g.rotateX(-Math.PI / 2);
      geom = g;
      break;
    }
    case SceneDatMeshKey.UNIT_CAPSULE: {
      const capsule = params?.kind.case === "capsule" ? params.kind.value : undefined;
      const radius = capsule?.radius ?? 0.5;
      const halfLength = capsule?.halfLength ?? 0.25;
      geom = new THREE.CapsuleGeometry(radius, Math.max(0, halfLength * 2), 6, 18);
      break;
    }
    case SceneDatMeshKey.UNIT_CONICAL_FRUSTUM: {
      const frustum = params?.kind.case === "conicalFrustum" ? params.kind.value : undefined;
      const rt = frustum?.radiusTop ?? 0.25;
      const rb = frustum?.radiusBottom ?? 0.5;
      const h = frustum?.height ?? 1;
      geom = new THREE.CylinderGeometry(rt, rb, h, 24, 1);
      break;
    }
    case SceneDatMeshKey.UNIT_TORUS: {
      const torus = params?.kind.case === "torus" ? params.kind.value : undefined;
      const minor = torus?.minorRadius ?? 0.15;
      const major = torus?.majorRadius ?? 0.45;
      geom = new THREE.TorusGeometry(major, minor, 14, 40);
      break;
    }
    case SceneDatMeshKey.UNIT_TRIANGLE: {
      const g = new THREE.BufferGeometry();
      const v = new Float32Array([
        -0.5,
        0,
        0.45,
        0.5,
        0,
        0.45,
        0,
        0,
        -0.55,
      ]);
      g.setAttribute("position", new THREE.BufferAttribute(v, 3));
      g.setIndex([0, 1, 2]);
      g.computeVertexNormals();
      geom = g;
      break;
    }
    case SceneDatMeshKey.UNIT_TETRAHEDRON:
      geom = new THREE.TetrahedronGeometry(0.6, 0);
      break;
    case SceneDatMeshKey.TREE_TRUNK:
      geom = new THREE.CylinderGeometry(0.18, 0.22, 1.2, 16, 1);
      break;
    case SceneDatMeshKey.TREE_CONE:
      geom = new THREE.ConeGeometry(0.75, 1.4, 18, 1);
      break;
    default:
      geom = new THREE.BoxGeometry(1, 1, 1);
      break;
  }

  // Primitives are cached and reused across reloads; don't dispose them on clear.
  geom.userData.__gravimera_cached = true;
  geometryCache.set(key, geom);
  return geom;
}

function materialColorFromKey(material: SceneDatPrimitiveMeshRef["material"] | undefined): Rgba {
  if (!material) {
    return { r: 0.85, g: 0.85, b: 0.9, a: 1 };
  }
  switch (material.kind.case) {
    case "buildBlock": {
      const index = material.kind.value.index >>> 0;
      // A tiny, deterministic palette based on index.
      const palette = [
        0xa9b1c3, 0x8aa6b8, 0xb8a78a, 0xa2b88a, 0xb88aa8, 0x8ab89a,
      ];
      const packed = palette[index % palette.length];
      const r = ((packed >>> 16) & 0xff) / 255;
      const g = ((packed >>> 8) & 0xff) / 255;
      const b = (packed & 0xff) / 255;
      return { r, g, b, a: 1 };
    }
    case "fenceStake":
      return { r: 0.42, g: 0.32, b: 0.22, a: 1 };
    case "fenceStick":
      return { r: 0.46, g: 0.36, b: 0.24, a: 1 };
    case "treeTrunk":
      return { r: 0.34, g: 0.25, b: 0.16, a: 1 };
    case "treeMain":
      return { r: 0.17, g: 0.45, b: 0.24, a: 1 };
    case "treeCrown":
      return { r: 0.14, g: 0.42, b: 0.22, a: 1 };
    default:
      return { r: 0.85, g: 0.85, b: 0.9, a: 1 };
  }
}

function renderPrimitive(
  primitive: SceneDatPrimitive,
  parent: THREE.Object3D,
  tint: Rgba,
) {
  if (primitive.kind.case === undefined) return;

  let meshKey: SceneDatMeshKey | null = null;
  let unlit = false;
  let base: Rgba = { r: 0.85, g: 0.85, b: 0.9, a: 1 };
  let params: SceneDatPrimitiveSolid["params"] | undefined;

  if (primitive.kind.case === "meshRef") {
    const meshRef = primitive.kind.value;
    meshKey = meshRef.mesh;
    base = materialColorFromKey(meshRef.material);
  } else if (primitive.kind.case === "solid") {
    const solid = primitive.kind.value;
    meshKey = solid.mesh;
    unlit = solid.unlit;
    base = rgbaFromColorDat(solid.color);
    params = solid.params;
  }

  if (meshKey === null) return;
  const color = mulRgba(base, tint);
  const geom = getPrimitiveGeometry(meshKey, params);
  const matBase = {
    color: new THREE.Color(color.r, color.g, color.b),
    transparent: color.a < 0.999,
    opacity: color.a,
  };

  const material = unlit
    ? new THREE.MeshBasicMaterial(matBase)
    : new THREE.MeshStandardMaterial({
        ...matBase,
        roughness: 0.92,
        metalness: 0.02,
      });

  const mesh = new THREE.Mesh(geom, material);
  parent.add(mesh);
}

type AnchorCache = Map<string, THREE.Matrix4>;

function buildAnchorCache(def: SceneDatObjectDef): AnchorCache {
  const out: AnchorCache = new Map();
  out.set("origin", new THREE.Matrix4().identity());
  for (const a of def.anchors) {
    out.set(a.name, transformToMatrix(a.transform));
  }
  return out;
}

function renderObjectDef(
  defId: bigint,
  defsById: Map<bigint, SceneDatObjectDef>,
  parent: THREE.Object3D,
  tint: Rgba,
  stack: Set<bigint>,
  anchorCacheById: Map<bigint, AnchorCache>,
  depth: number,
) {
  if (depth > 16) return;
  if (stack.has(defId)) return;
  stack.add(defId);

  const def = defsById.get(defId);
  if (!def) {
    // Placeholder.
    const g = new THREE.BoxGeometry(1, 1, 1);
    const m = new THREE.MeshStandardMaterial({ color: 0xff4d4d, roughness: 0.8 });
    parent.add(new THREE.Mesh(g, m));
    stack.delete(defId);
    return;
  }

  let anchorCache = anchorCacheById.get(defId);
  if (!anchorCache) {
    anchorCache = buildAnchorCache(def);
    anchorCacheById.set(defId, anchorCache);
  }

  for (const part of def.parts) {
    const partObj = new THREE.Object3D();
    parent.add(partObj);

    const offsetMat = transformToMatrix(part.transform);
    let partMat = offsetMat;

    if (part.attachment) {
      const parentAnchorMat = anchorCache.get(part.attachment.parentAnchor) ?? anchorCache.get("origin")!;
      let childAnchorMat = new THREE.Matrix4().identity();
      if (part.kind.case === "objectRef") {
        const childId = uuidToU128(part.kind.value);
        if (childId !== null) {
          const childDef = defsById.get(childId);
          if (childDef) {
            let childAnchors = anchorCacheById.get(childId);
            if (!childAnchors) {
              childAnchors = buildAnchorCache(childDef);
              anchorCacheById.set(childId, childAnchors);
            }
            childAnchorMat =
              childAnchors.get(part.attachment.childAnchor) ?? childAnchors.get("origin")!;
          }
        }
      }

      // parent_anchor * offset * inv(child_anchor)
      partMat = parentAnchorMat
        .clone()
        .multiply(offsetMat)
        .multiply(childAnchorMat.clone().invert());
    }

    setObjectLocalMatrix(partObj, partMat);

    switch (part.kind.case) {
      case "objectRef": {
        const childId = uuidToU128(part.kind.value);
        if (childId !== null) {
          renderObjectDef(
            childId,
            defsById,
            partObj,
            tint,
            stack,
            anchorCacheById,
            depth + 1,
          );
        }
        break;
      }
      case "primitive":
        renderPrimitive(part.kind.value, partObj, tint);
        break;
      case "model": {
        const g = new THREE.BoxGeometry(1, 1, 1);
        const m = new THREE.MeshStandardMaterial({
          color: 0x4d7cff,
          roughness: 0.9,
          metalness: 0.05,
        });
        partObj.add(new THREE.Mesh(g, m));
        break;
      }
      default:
        break;
    }
  }

  stack.delete(defId);
}

function renderTerrain(def: TerrainDefV1 | undefined, parent: THREE.Object3D) {
  const mesh = def?.mesh;
  const mat = def?.material;

  const sizeX = mesh?.sizeXM ?? 40;
  const sizeZ = mesh?.sizeZM ?? 40;
  const segX = Math.max(1, Math.min(512, mesh?.subdivX ?? 1));
  const segZ = Math.max(1, Math.min(512, mesh?.subdivZ ?? 1));

  const g = new THREE.PlaneGeometry(sizeX, sizeZ, segX, segZ);
  g.rotateX(-Math.PI / 2);
  g.translate(0, 0, 0);

  const base = {
    r: clamp01(mat?.baseColorR ?? 0.16),
    g: clamp01(mat?.baseColorG ?? 0.17),
    b: clamp01(mat?.baseColorB ?? 0.2),
    a: clamp01(mat?.baseColorA ?? 1),
  };

  const m = (mat?.unlit ?? false)
    ? new THREE.MeshBasicMaterial({
        color: new THREE.Color(base.r, base.g, base.b),
        transparent: base.a < 0.999,
        opacity: base.a,
        side: THREE.DoubleSide,
      })
    : new THREE.MeshStandardMaterial({
        color: new THREE.Color(base.r, base.g, base.b),
        transparent: base.a < 0.999,
        opacity: base.a,
        roughness: clamp01(mat?.roughness ?? 0.9),
        metalness: clamp01(mat?.metallic ?? 0.0),
        side: THREE.DoubleSide,
      });

  const plane = new THREE.Mesh(g, m);
  plane.receiveShadow = false;
  parent.add(plane);
}

function clearWorld() {
  // Keep grid helper at index 0.
  while (worldRoot.children.length > 1) {
    const child = worldRoot.children[worldRoot.children.length - 1];
    worldRoot.remove(child);
    child.traverse((obj: THREE.Object3D) => {
      const asMesh = obj as THREE.Mesh;
      if (asMesh.isMesh) {
        if (asMesh.geometry && !(asMesh.geometry as any).userData?.__gravimera_cached) {
          asMesh.geometry.dispose();
        }
        const mat = asMesh.material as unknown;
        if (Array.isArray(mat)) {
          for (const m of mat) (m as THREE.Material).dispose?.();
        } else if (mat && (mat as THREE.Material).dispose) {
          (mat as THREE.Material).dispose();
        }
      }
    });
  }
}

function frameCameraToObject(obj: THREE.Object3D) {
  const box = new THREE.Box3().setFromObject(obj);
  if (box.isEmpty()) return;
  const center = new THREE.Vector3();
  const size = new THREE.Vector3();
  box.getCenter(center);
  box.getSize(size);

  const radius = Math.max(1.0, size.length() * 0.55);
  controls.target.copy(center);
  camera.position.copy(center).add(new THREE.Vector3(radius * 1.25, radius * 0.9, radius * 1.25));
  camera.near = Math.max(0.01, radius / 200);
  camera.far = Math.max(200, radius * 20);
  camera.updateProjectionMatrix();
  controls.update();
}

function renderScene(sceneDat: SceneDat, terrainDef: TerrainDefV1 | undefined) {
  clearWorld();
  renderTerrain(terrainDef, worldRoot);

  const defsById = new Map<bigint, SceneDatObjectDef>();
  for (const def of sceneDat.defs) {
    const id = uuidToU128(def.objectId);
    if (id !== null) defsById.set(id, def);
  }

  const unitsPerMeter = Math.max(1, sceneDat.unitsPerMeter || 100);
  const anchorCacheById = new Map<bigint, AnchorCache>();
  const stack = new Set<bigint>();

  let count = 0;
  for (const inst of sceneDat.instances) {
    const baseId = uuidToU128(inst.baseObjectId);
    if (baseId === null) continue;

    const forms =
      inst.forms.length > 0
        ? inst.forms.map(uuidToU128).filter((x): x is bigint => x !== null)
        : [];
    if (forms.length === 0) forms.push(baseId);
    const active = inst.activeForm >= 0 && inst.activeForm < forms.length ? inst.activeForm : 0;
    const prefabId = forms[active] ?? baseId;

    const root = new THREE.Object3D();
    worldRoot.add(root);

    const pos = new THREE.Vector3(
      inst.xUnits / unitsPerMeter,
      inst.yUnits / unitsPerMeter,
      inst.zUnits / unitsPerMeter,
    );
    const rot = new THREE.Quaternion(inst.rotX, inst.rotY, inst.rotZ, inst.rotW);
    const scl = new THREE.Vector3(
      inst.scaleX?.value ?? 1,
      inst.scaleY?.value ?? 1,
      inst.scaleZ?.value ?? 1,
    );
    root.position.copy(pos);
    root.quaternion.copy(rot);
    root.scale.copy(scl);

    const tint = inst.tint ? rgbaFromColorDat(inst.tint) : { r: 1, g: 1, b: 1, a: 1 };
    renderObjectDef(prefabId, defsById, root, tint, stack, anchorCacheById, 0);
    count += 1;
  }

  frameCameraToObject(worldRoot);
  logLine(
    logEl,
    `Loaded scene: defs=${sceneDat.defs.length}, instances=${count}, units_per_meter=${unitsPerMeter}`,
  );
}

async function fetchBytes(url: string): Promise<Uint8Array> {
  const resp = await fetch(url);
  if (!resp.ok) {
    throw new Error(`HTTP ${resp.status} ${resp.statusText}`);
  }
  return new Uint8Array(await resp.arrayBuffer());
}

async function loadFiles() {
  const sceneFile = sceneFileEl.files?.[0];
  if (!sceneFile) {
    logLine(logEl, "Pick a scene.grav file.");
    return;
  }

  const sceneBytes = new Uint8Array(await sceneFile.arrayBuffer());
  let sceneDat: SceneDat;
  try {
    sceneDat = fromBinary(SceneDatSchema, sceneBytes);
  } catch (err) {
    logLine(logEl, `Failed to decode scene.grav: ${String(err)}`);
    return;
  }

  let terrainDef: TerrainDefV1 | undefined = undefined;
  const terrainFile = terrainFileEl.files?.[0];
  if (terrainFile) {
    const terrainBytes = new Uint8Array(await terrainFile.arrayBuffer());
    try {
      const terrainDat = fromBinary(SceneTerrainDatSchema, terrainBytes);
      terrainDef = terrainDat.terrainDef;
    } catch (err) {
      logLine(logEl, `Failed to decode terrain.grav (continuing without terrain): ${String(err)}`);
    }
  }

  renderScene(sceneDat, terrainDef);
}

async function autoLoadWastelandScene() {
  const sceneUrl = "/assets/scene_wasteland/scene.grav";
  const terrainUrl = "/assets/scene_wasteland/terrain.grav";

  logLine(logEl, `Auto-loading default scene from ${sceneUrl} ...`);

  let sceneDat: SceneDat;
  try {
    sceneDat = fromBinary(SceneDatSchema, await fetchBytes(sceneUrl));
  } catch (err) {
    logLine(logEl, `Auto-load failed: could not load ${sceneUrl}: ${String(err)}`);
    return;
  }

  let terrainDef: TerrainDefV1 | undefined = undefined;
  try {
    const terrainDat = fromBinary(SceneTerrainDatSchema, await fetchBytes(terrainUrl));
    terrainDef = terrainDat.terrainDef;
  } catch (err) {
    logLine(logEl, `Auto-load: could not load ${terrainUrl} (continuing without terrain): ${String(err)}`);
  }

  renderScene(sceneDat, terrainDef);
}

loadBtnEl.addEventListener("click", () => {
  void loadFiles();
});

let lastCanvasW = 0;
let lastCanvasH = 0;
let lastDpr = 0;

function resizeToCanvasIfNeeded() {
  // Use the *actual* canvas CSS size instead of `window.innerWidth/innerHeight`.
  // On some mobile in-app browsers, orientation changes don't reliably fire `resize`,
  // which can lead to a stretched projection matrix.
  const w = Math.max(1, Math.floor(canvas.clientWidth));
  const h = Math.max(1, Math.floor(canvas.clientHeight));
  const dpr = Math.min(2, window.devicePixelRatio || 1);

  if (w === lastCanvasW && h === lastCanvasH && dpr === lastDpr) return;
  lastCanvasW = w;
  lastCanvasH = h;
  lastDpr = dpr;

  renderer.setPixelRatio(dpr);
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}

window.addEventListener("resize", resizeToCanvasIfNeeded);
resizeToCanvasIfNeeded();

void autoLoadWastelandScene();

function animate() {
  requestAnimationFrame(animate);
  resizeToCanvasIfNeeded();
  applyCameraKeyControls(clock.getDelta());
  controls.update();
  renderer.render(threeScene, camera);
}
animate();
