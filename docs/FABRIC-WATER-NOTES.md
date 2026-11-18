# Fabric: сверка кода 26.3 для воды выше моря (Ф6-В0)

**Статус: черновик спайка, 2026-11-18.** План — `docs/PLAN-WATER-FABRIC.md` §5 Ф6-В0. Делать после влива PR #51 (Ф1–Ф5, d8b2934). Код не написан, этот файл — итог шага В0: где в 26.3 внедрять заливку по колонкам и что проверить локально/в CI. Ваниль 26.3 идёт без обфускации (Loom без ремапа), декомпилятор — Vineflower.

**Вывод спайка: основная точка внедрения — синхронная заливка `NoiseBasedChunkGenerator` (в 1.21 — `doFill`), запасная — фича на шаге `RAW_GENERATION`.** Причины выбора — в конце.

Как воспроизвести локально (нужен `server.jar` 26.3, Fabric берёт его из `piston-meta`):

```bash
# 1. скачать server.jar как в fabric.yml (нужен доступ к piston-meta.mojang.com)
mv=$(grep '^minecraft_version=' fabric/gradle.properties | cut -d= -f2)
url=$(curl -fsS https://piston-meta.mojang.com/mc/game/version_manifest_v2.json | jq -r --arg v "$mv" '.versions[]|select(.id==$v).url')
srv=$(curl -fsS "$url" | jq -r '.downloads.server.url')
curl -fsS -o /tmp/server.jar "$srv"

# 2. распаковать внутренний jar (как tools/fabric_resources.py)
python3 -c "import zipfile,io; z=zipfile.ZipFile('/tmp/server.jar'); inner=[n for n in z.namelist() if n.startswith('META-INF/versions/') and n.endswith('.jar')]; open('/tmp/minecraft-26.3.jar','wb').write(z.read(inner[0])) if inner else None"

# 3. декомпиляция Vineflower (job fabric.yml делает то же на раннере)
java -jar /tmp/vineflower.jar /tmp/minecraft-26.3.jar /tmp/decompiled-26.3
grep -RIn "NoiseBasedChunkGenerator\|doFill\|fillFromNoise\|ProtoChunk\|SurfaceSystem\|NoiseBasedAquifer\|markPosForPostprocessing\|skip_above" /tmp/decompiled-26.3 | head -100
```

Без локального `server.jar` (как в этом дев-контейнере без Java/доступа к Mojang) сверка идёт по памяти кода 1.18–1.21 и по открытым исходникам Horizon `66f3981` (порт Java). Ниже — гипотезы для проверки в job `fabric.yml`.

## 1. Синхронная заливка в `NoiseBasedChunkGenerator`

*Что искать:* `NoiseBasedChunkGenerator` — `ChunkGenerator` ванили, который строит террейн из шума. В 1.21.1 синхронный проход назывался `doFill(Blender, StructureManager, RandomState, ChunkAccess, ...)` и вызывался из `fillFromNoise`, который уже асинхронный (`CompletableFuture<ChunkAccess>`). План §2 п.1 запрещает целиться в `fillFromNoise` — заливка должна идти в `doFill`.

*Гипотеза для 26.3:* имя и сигнатура почти те же — `private ChunkAccess doFill(ChunkAccess chunk, StructureManager, Blender, RandomState, ...)` или `protected void fill(...)`. Сигнатура обязана содержать `RandomState` (там сид и `Sampler` ванильного шума), поэтому `LithoFields.Binding` (сид+режим) можно достать как в `LithoFields.ColumnField` — миксин на `RandomStateMixin` уже делает `MODES.put(state, mode)`. Mixin-цель — `INVOKE` в конце `doFill` после цикла по `chunk` (16×16×height) и до `return chunk`. Проверяется `instanceof ServerLevel`/`NoiseBasedChunkGenerator` и наличие пресета `litospherity:*` (версия R2).

*Что записать в job:* точное FQN `net.minecraft.world.level.chunk.NoiseBasedChunkGenerator`, имя метода, дескриптор (`(Lnet/minecraft/world/level/levelgen/RandomState;...)Lnet/minecraft/world/level/chunk/ChunkAccess;`), есть ли `RandomState` в параметрах/локальных, и что `ChunkAccess` — это `ProtoChunk`/`ChunkAccess` (ниже).

## 2. `ProtoChunk.setBlockState` и карты высот

*Что искать:* `ProtoChunk implements ChunkAccess, LevelHeightAccessor` — `setBlockState(BlockPos, BlockState, boolean)` обновляет `LevelChunkSection` и heightmaps `Heightmap.Types.OCEAN_FLOOR_WG`, `WORLD_SURFACE_WG`, `MOTION_BLOCKING` и др.

*Гипотеза (совпадает с Horizon):* внутри `setBlockState` вызывается `heightmaps.get(type).update(x,y,z, state)` если `state.isCollisionShapeFullBlock` или `state.getFluidState().isSource()`. Для воды на `level-1` это означает, что `WORLD_SURFACE_WG` поднимется до воды, а `OCEAN_FLOOR_WG` останется на дне — как у ванильного океана. Если реализация 26.3 не трогает heightmap для воды (редкий случай), то в `LithoWater` после цикла нужно вызвать `chunk.markPosForPostprocessing` или `Heightmap.update` вручную (проверить в декомпиле: есть ли `this.heightmaps` поле).

*Что записать:* строки `heightmaps` в `ProtoChunk.setBlockState`, обновляются ли `OCEAN_FLOOR_WG`/`WORLD_SURFACE_WG` при `Blocks.WATER.defaultBlockState()`, и нужен ли ручной `update`.

## 3. `SurfaceSystem` и высота воды в колонке

*Что искать:* `net.minecraft.world.level.levelgen.SurfaceSystem` (ранее `SurfaceRules`/`MaterialRule`) — `buildSurface(RandomState, BiomeSource, Registry<Biome>, NoiseChunk, ChunkAccess, ...)` или `applyMaterialRule`.

*Гипотеза:* ваниль находит воду как `min(surfaceY + 1, seaLevel)` для океана и как `chunk.getFluidState(pos).is(Fluids.WATER)` для рек (но у нас русло уже залито на шаге NOISE, см. §3 плана). Для Fabric нам важно противоположное: `SurfaceSystem` идёт **после** NOISE (раздел 3 плана: «Шаг SURFACE идёт после NOISE и видит воду»), поэтому дно под водой получит `minecraft:water` → `sand`/`gravel` без травы. Проверяется условием `if (blockState.is(Blocks.WATER))` в `SurfaceSystem` — тогда ставится подводный материал.

*Что записать:* точную строку `if (isWater)` в `SurfaceSystem`, и что заливка на NOISE достаточна, чтобы поверхность стала подводной.

## 4. Карверы и `hasDisallowedLiquid`

*Что искать:* `net.minecraft.world.level.levelgen.carver.Carver` / `CaveCarver` — метод `hasDisallowedLiquid(ChunkAccess, BlockPos)` или `canReplaceWithWater`.

*Гипотеза (1.18–1.21):* карвер проверяет `if (state.getFluidState().isSourceOf(Fluids.WATER)) return false` в радиусе 1–2 блоков, чтобы не вскрывать озеро. Наши высокие озёра и реки должны пройти ту же проверку — иначе карвер прорежет дыру в дно. Декомпиль должен показать `FluidState` проверку в `CarverConfiguration`/`CarvingMask`.

*Что записать:* есть ли проверка жидкости, и блокирует ли она карвер над `level > 63` (если нет — затычка `LithoWater` уровня `bed-2` и герметизация `WaterSealFeature` это закрывают, см. риски плана).

## 5. `markPosForPostprocessing` и тики жидкости

*Что искать:* `ChunkAccess.markPosForPostprocessing(BlockPos)` и `LevelChunk.updateBlockTick` / `scheduleFluidTick`.

*Гипотеза:* `markPosForPostprocessing` помечает колонку для `PostProcessing` на переходе `ProtoChunk → LevelChunk` (создаётся `LevelChunk` из проточанка). Для воды ваниль ставит `FluidTick` на `level-1` если `level > seaLevel` и сосед — воздух (перекат). План §3 п.3 даёт два варианта ступеней: статичный перекат (ставим `WATER level=7` без тика) и живые перекаты (`markPosForPostprocessing` + `scheduleTick(Fluids.WATER, 5)`). Выбор — по замеру TPS в Ф6-В6 (≤50 мс/тик).

*Что записать:* что делает `markPosForPostprocessing` в 26.3 (ставит ли `pendingBlockEntitiesTickers` или `postProcessing` список), и как ваниль планирует `FluidTick` для воды выше моря (если есть — можно переиспользовать).

## 6. Быстрый путь `skip_above_y` в `NoiseBasedAquifer`

*Что искать:* `net.minecraft.world.level.levelgen.NoiseBasedAquifer` — поле `int skip_above_y` или `int yFrom` в `computeFluid`/`getFluidState`.

*Гипотеза (причина 5 раздела 2 плана, horizon#16424 п.1):* Horizon пишет воздух над `skip_above_y = surface + ~20..30` без вызова `FluidPicker.compute`. В Java 26.3 то же: `if (y > skipAbove) return Fluids.EMPTY`. Это подтверждает, что аквифер нельзя usar основным — глубокие колонки озера на y 150 будут отброшены, и заливка должна идти вне аквифера (наш NOISE-проход).

*Что записать:* имя поля/константы, её значение (обычно `Math.max(surfaceY + 8, seaLevel + 20)`), и что она режет воду выше моря.

## 7. Что C2ME заменяет в шумовом шаге

*Что искать:* мод `C2ME` (`com.ishland.c2me`) — миксины `mixin.chunkio`, `mixin.threaded_anvil_chunk_storage`.

*Гипотеза:* C2ME заменяет `ThreadedAnvilChunkStorage` и `ChunkMap` на `CompletableFuture`-пайплайн с пулом воркеров, но не трогает сам `NoiseBasedChunkGenerator.doFill` — он вызывается из `ChunkStatus.NOISE` через `ChunkGenerator.fillFromNoise`. Поэтому наш `NoiseFillMixin` на `doFill` совместим с C2ME, а замена аквифера (отвергнута) — нет. Проверяется `grep -R c2me` в декомпиле с модом в `mods/`.

*Что записать:* список миксинов C2ME, трогают ли они `NoiseBasedChunkGenerator`/`ProtoChunk.setBlockState`, и что заливка остаётся синхронной.

## Решение по точке внедрения

- **Основная:** `NoiseBasedChunkGenerator.doFill` (или её точное имя в 26.3) — конец метода, `INJECT at RETURN`, `cancellable = false`. Условия: `RandomState` доступен, `ChunkAccess` — `ProtoChunk`, `SurfaceSystem` после NOISE увидит воду, C2ME не обходит вызов.
- **Запасная:** фича `litospherity:water_seal` на шаге `RAW_GENERATION` (раздел 3 плана). Включается если: (а) синхронной точки нет, (б) C2ME делает `doFill` асинхронным, (в) `setBlockState` не обновляет heightmap и требует ручного `Heightmap.update` вне чанка. Цена запасной — ручная смена `grass_block → dirt` под водой (повтор логики `SurfaceSystem`).
- **Отклонено:** миксин на `NoiseBasedAquifer.FluidPicker.compute` — по причинам 1–5 раздела 2 (сетка 16×12×16, `+8` условие, барьеры, затопленные пещеры, `skip_above_y`).

Следующий шаг — Ф6-В1: `GenVersion::V1DevR2` и `Generator::water_column` (см. `docs/PLAN-WATER-FABRIC.md` §5).

---
*Как проверить в CI:* job `fabric` после сборки `litho-ffi` может запустить Vineflower-джоб (кэшировать `server.jar` по `minecraft_version`), прогнать `grep` по декомпилу и выложить этот файл как артефакт; провал — если имя `doFill` не найдено. Локально — команды выше.
