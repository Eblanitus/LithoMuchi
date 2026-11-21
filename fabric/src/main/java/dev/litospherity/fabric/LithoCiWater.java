package dev.litospherity.fabric;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.status.ChunkStatus;

/**
 * Ф6-В5 (PLAN-WATER-FABRIC §5): CI «вода не стекает по склону».
 *
 * <p>Режим {@code -Dlitospherity.ci=water-<режим>}. Читает {@code run/water-points.txt}
 * (вывод {@code litho-sandbox water-points}), генерирует вокруг каждого места 5×5 чанков,
 * держит их загруженными 400 тиков и считает метрики. Провал job'а — {@code RESULT: FAIL}.
 */
final class LithoCiWater {
    private LithoCiWater() {}

    /** Сколько тиков ждать, чтобы тики жидкости прошли (план: 400). */
    private static final int TICKS = 400;
    /** Радиус области вокруг места, чанки: 5×5. */
    private static final int RADIUS = 2;

    /** Сгенерированные чанки окна: пары (x, z) в одной long. */
    private static final Set<Long> CHUNKS = new LinkedHashSet<>();

    static void run(MinecraftServer server, String mode) {
        LithoWaterFeature.STEPS.set(0);
        LithoWaterFeature.STATIC_MARKS_REMOVED.set(0);
        StaticRiverMask.MARKS_REMOVED.set(0);
        StaticRiverMask.TICKS_REMOVED.set(0);
        LithoWaterFeature.COLUMNS.set(0);
        LithoWaterFeature.SEALS.set(0);
        LithoWaterFeature.CHUNKS.set(0);
        List<String> report = new ArrayList<>();
        try {
            ServerLevel level = server.overworld();
            List<int[]> points = readPoints();
            report.add("water: режим " + mode + " сид " + level.getSeed() + " мест " + points.size());
            if (points.isEmpty()) {
                report.add("FAIL: нет файла run/water-points.txt или в нём нет мест");
                finish(server, report, false);
                return;
            }
            LithoNative.Generator gen = LithoFields.generator(LithoFields.binding(level.getChunkSource().randomState()));
            long t0 = System.nanoTime();
            Set<Long> generated = CHUNKS;
            generated.clear();
            for (int[] p : points) {
                int cx = p[0] >> 4;
                int cz = p[1] >> 4;
                for (int dx = -RADIUS; dx <= RADIUS; dx++) {
                    for (int dz = -RADIUS; dz <= RADIUS; dz++) {
                        level.getChunk(cx + dx, cz + dz, ChunkStatus.FULL, true);
                        if (generated.add((((long) (cx + dx)) << 32) ^ Integer.toUnsignedLong(cz + dz))) {
                            level.setChunkForced(cx + dx, cz + dz, true);
                        }
                    }
                }
            }
            double ms = (System.nanoTime() - t0) / 1e6;
            report.add(String.format("вода: сгенерировано %d чанков за %.0f мс, %.1f мс/чанк",
                    generated.size(), ms, ms / Math.max(1, generated.size())));
            report.add("снято меток postProcess в статичной воде: " + LithoWaterFeature.STATIC_MARKS_REMOVED.get());
            report.add("статичная река перед FULL: меток снято " + StaticRiverMask.MARKS_REMOVED.get()
                    + ", отложенных тиков воды снято " + StaticRiverMask.TICKS_REMOVED.get());
            long generationSeals = LithoWaterFeature.SEALS.get();
            long chunksWithWater = LithoWaterFeature.CHUNKS.get();
            long sourcesBefore = countWater(level, true);
            long flowingBefore = countWater(level, false);
            FLOWING_BEFORE.clear();
            collectFlowing(level, FLOWING_BEFORE);
            var watches = WaterWatch.fromEnvironment(level, report);
            long[] tickStart = {0L}, totalTickNanos = {0L}, maxTickNanos = {0L};
            int[] measuredTicks = {0};
            boolean[] done = {false};
            ServerTickEvents.START_SERVER_TICK.register(s -> {
                if (!done[0]) tickStart[0] = System.nanoTime();
            });
            ServerTickEvents.END_SERVER_TICK.register(s -> {
                if (done[0] || tickStart[0] == 0L) return;
                long dt = System.nanoTime() - tickStart[0];
                totalTickNanos[0] += dt;
                maxTickNanos[0] = Math.max(maxTickNanos[0], dt);
                int tick = ++measuredTicks[0];
                for (WaterWatch watch : watches) watch.observe(level, tick, report);
                if (tick < TICKS) return;
                done[0] = true;
                double avgTickMs = totalTickNanos[0] / (double) tick / 1_000_000.0;
                double maxTickMs = maxTickNanos[0] / 1_000_000.0;
                measure(server, level, gen, points, report, generationSeals, chunksWithWater, sourcesBefore, flowingBefore, ms / Math.max(1, generated.size()), avgTickMs, maxTickMs);
            });
        } catch (Throwable t) {
            report.add("FAIL: " + t);
            LithoMod.LOG.error("Litospherity CI water", t);
            finish(server, report, false);
        }
    }

    /** CI only. Read the loaded, already scored chunks; never load a chunk or
     * schedule a tick. For a leak, retain the LAST dry neighbourhood and the
     * first wet one rather than inferring the source from tick 400. */
    private static final class WaterWatch {
        private final BlockPos pos;
        private String previous;
        private boolean found;

        private WaterWatch(BlockPos pos) { this.pos = pos; }

        static List<WaterWatch> fromEnvironment(ServerLevel level, List<String> report) {
            var watches = new ArrayList<WaterWatch>();
            String spec = System.getenv("LITHO_CI_WATER_WATCH");
            if (spec == null || spec.isBlank()) return watches;
            for (String point : spec.split(";")) {
                String[] xyz = point.split(",");
                if (xyz.length != 3) throw new IllegalArgumentException("water watch: " + point);
                var pos = new BlockPos(Integer.parseInt(xyz[0]), Integer.parseInt(xyz[1]), Integer.parseInt(xyz[2]));
                boolean loaded = true;
                for (int dx : new int[]{-1, 1}) for (int dz : new int[]{-1, 1}) {
                    int cx = (pos.getX() + dx) >> 4, cz = (pos.getZ() + dz) >> 4;
                    loaded &= CHUNKS.contains(((long) cx << 32) ^ Integer.toUnsignedLong(cz));
                }
                if (!loaded) { report.add("watch вне окон: " + point); continue; }
                var watch = new WaterWatch(pos);
                watches.add(watch);
                watch.observe(level, 0, report);
            }
            return watches;
        }

        void observe(ServerLevel level, int tick, List<String> report) {
            if (found) return;
            StringBuilder frame = new StringBuilder();
            // Each square: rows N->S, columns W->E. S=source, 1..8=flow,
            // .=air, #=solid/other. Include overhead flow and the bed below.
            for (int dy : new int[]{0, 1, -1}) {
                if (dy != 0) frame.append(' ');
                for (int dz = -1; dz <= 1; dz++) {
                    if (dz != -1) frame.append('/');
                    for (int dx = -1; dx <= 1; dx++) {
                        BlockState state = level.getBlockState(pos.offset(dx, dy, dz));
                        frame.append(!containsWater(state) ? (state.isAir() ? '.' : '#') :
                                state.getFluidState().isSource() ? 'S' : (char)('0' + state.getFluidState().getAmount()));
                    }
                }
            }
            found = containsWater(level.getBlockState(pos));
            if (tick == 0 || found || tick == TICKS) {
                report.add("watch " + pos.toShortString() + " tick=" + tick + " wet=" + found
                        + (found && previous != null ? " предыдущий=" + previous : "") + " сейчас=" + frame);
            }
            previous = frame.toString();
        }
    }

    private static void measure(MinecraftServer server, ServerLevel level, LithoNative.Generator gen,
                                List<int[]> points, List<String> report, long generationSeals, long chunksWithWater,
                                long sourcesBefore, long flowingBefore, double msPerChunk, double avgTickMs, double maxTickMs) {
        boolean ok = true;
        try {
            long flowingAfter = countWater(level, false);
            long sourcesAfter = countWater(level, true);
            long growthAll = flowingAfter - flowingBefore;
            long seals = generationSeals;
            double sealsPerChunk = chunksWithWater == 0 ? 0.0 : seals / (double) chunksWithWater;

            // Где выросла текущая вода: первые места с ядром колонки и соседей,
            // чтобы разбирать рост по адресу (ядро считается локально).
            Set<Long> flowingAfterSet = new LinkedHashSet<>();
            collectFlowing(level, flowingAfterSet);
            // Группы новой текущей воды (связность по 6 соседям): размер, рамка и
            // верхняя точка с ядром колонки и блоками вокруг. Источник роста ищется
            // у верхней точки группы, а первые 12 мест показывали одну группу.
            Set<Long> fresh = new LinkedHashSet<>();
            for (long key : flowingAfterSet) if (!FLOWING_BEFORE.contains(key)) fresh.add(key);
            java.util.List<long[]> groups = new java.util.ArrayList<>();
            Set<Long> seen = new java.util.HashSet<>();
            for (long key : fresh) {
                if (!seen.add(key)) continue;
                java.util.ArrayDeque<Long> queue = new java.util.ArrayDeque<>();
                queue.add(key);
                long size = 0;
                int minX = Integer.MAX_VALUE, minY = Integer.MAX_VALUE, minZ = Integer.MAX_VALUE;
                int maxX = Integer.MIN_VALUE, maxYb = Integer.MIN_VALUE, maxZ = Integer.MIN_VALUE;
                long topKey = key;
                while (!queue.isEmpty()) {
                    long cur = queue.poll();
                    size++;
                    BlockPos p = BlockPos.of(cur);
                    minX = Math.min(minX, p.getX()); maxX = Math.max(maxX, p.getX());
                    minY = Math.min(minY, p.getY());
                    minZ = Math.min(minZ, p.getZ()); maxZ = Math.max(maxZ, p.getZ());
                    if (p.getY() > maxYb) { maxYb = p.getY(); topKey = cur; }
                    for (BlockPos n : new BlockPos[] {p.above(), p.below(), p.east(), p.west(), p.north(), p.south()}) {
                        long nk = n.asLong();
                        if (fresh.contains(nk) && seen.add(nk)) queue.add(nk);
                    }
                }
                groups.add(new long[] {size, topKey, minX, minY, minZ, maxX, maxYb, maxZ});
            }
            groups.sort((a, b) -> Long.compare(b[0], a[0]));
            // Чужая вода: группа, у верхней точки которой ни в колонке, ни у 4
            // соседей нет нашей воды (уровень ядра ≥ 63). Наша заливка ставит воду
            // только в такие колонки, значит верх группы без них — ванильный
            // родник или водоносный слой на склоне (CI 914e370: водопад 446 блоков
            // с y = 102 у (-8410, 1766), ядро вокруг пустое). Ванильная вода течёт
            // и без мода, в рост нашей воды её не считаем, но печатаем.
            long foreign = 0;
            boolean[] foreignGroup = new boolean[groups.size()];
            for (int g = 0; g < groups.size(); g++) {
                BlockPos top = BlockPos.of(groups.get(g)[1]);
                boolean ours = false;
                for (int[] d : new int[][] {{0, 0}, {1, 0}, {-1, 0}, {0, 1}, {0, -1}}) {
                    int lv = gen.water(new double[] {top.getX() + d[0]}, new double[] {top.getZ() + d[1]}).level()[0];
                    if (lv != LithoNative.WaterColumns.NONE && lv >= LithoWaterFeature.SEA_LEVEL) ours = true;
                }
                if (!ours) {
                    foreignGroup[g] = true;
                    foreign += groups.get(g)[0];
                }
            }
            long growth = Math.max(0, growthAll - foreign);
            double growthPct = sourcesAfter == 0 ? 0.0 : 100.0 * growth / (double) sourcesAfter;
            report.add(String.format("новая текущая: %d блоков, групп %d", fresh.size(), groups.size()));
            for (int g = 0; g < Math.min(10, groups.size()); g++) {
                long[] gr = groups.get(g);
                BlockPos top = BlockPos.of(gr[1]);
                StringBuilder line = new StringBuilder(String.format(
                        "группа %d%s: %d блоков, x %d..%d y %d..%d z %d..%d, верх (%d, %d, %d)",
                        g + 1, foreignGroup[g] ? " (чужая, не наша вода)" : "", gr[0], gr[2], gr[5], gr[3], gr[6], gr[4], gr[7], top.getX(), top.getY(), top.getZ()));
                for (int[] d : new int[][] {{0, 0}, {1, 0}, {-1, 0}, {0, 1}, {0, -1}}) {
                    LithoNative.WaterColumns c = gen.water(new double[] {top.getX() + d[0]}, new double[] {top.getZ() + d[1]});
                    int lv = c.level()[0];
                    line.append(String.format(" [%d,%d] level=%s kind=%d", d[0], d[1],
                            lv == LithoNative.WaterColumns.NONE ? "-" : Integer.toString(lv), c.kind()[0]));
                }
                line.append(" | вокруг верха:");
                for (BlockPos n : new BlockPos[] {top.above(), top.east(), top.west(), top.south(), top.north()}) {
                    BlockState st = level.getBlockState(n);
                    String name = containsWater(st) ? (st.getFluidState().isSource() ? "water-src" : "water-flow" + st.getFluidState().getAmount())
                            : net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(st.getBlock()).getPath();
                    line.append(' ').append(name);
                }
                report.add(line.toString());
            }

            var dryBlocks = new java.util.TreeMap<String, Integer>();
            long leaks = 0;
            long wet = 0;
            long dry = 0;
            for (int[] p : points) {
                for (int x = (p[0] & ~15) - RADIUS * 16; x < (p[0] & ~15) + (RADIUS + 1) * 16; x++) {
                    for (int z = (p[1] & ~15) - RADIUS * 16; z < (p[1] & ~15) + (RADIUS + 1) * 16; z++) {
                        LithoNative.WaterColumns core = gen.water(new double[] {x}, new double[] {z});
                        int top = core.level()[0];
                        if (top == LithoNative.WaterColumns.NONE || top <= LithoWaterFeature.SEA_LEVEL) continue;
                        int y = top - 1;
                        BlockState state = level.getBlockState(new BlockPos(x, y, z));
                        if (containsWater(state)) {
                            wet++;
                        } else {
                            dry++;
                            dryBlocks.merge(state.getBlock().toString(), 1, Integer::sum);
                        }
                        // Протечка: вода выше поверхности самой колонки. Вода
                        // ядра глубже верха колонки — не протечка: ядро кладёт
                        // источник только туда, где вырез опустил дно под
                        // зеркало, а выше могут остаться воздух или берег.
                        // Раньше окно `top-2…top` считало такие блоки воды
                        // протечкой, хотя это наша же вода (§4 п. 4, Д-127).
                        for (int yy = Math.max(LithoWaterFeature.SEA_LEVEL + 1, y - 2); yy <= y + 1; yy++) {
                            if (!containsWater(level.getBlockState(new BlockPos(x, yy, z)))) continue;
                            boolean coreHere = false;
                            for (int dx = -1; dx <= 1 && !coreHere; dx++) {
                                for (int dz = -1; dz <= 1 && !coreHere; dz++) {
                                    LithoNative.WaterColumns near = gen.water(new double[] {x + dx}, new double[] {z + dz});
                                    if (near.level()[0] == LithoNative.WaterColumns.NONE) continue;
                                    // Наша вода колонки (x+dx, z+dz) — от зеркала
                                    // вниз до дна; выше зеркала её быть не должно.
                                    coreHere = yy <= near.level()[0] - 1 && yy >= near.bed()[0] - 2;
                                }
                            }
                            if (!coreHere) {
                                boolean atSurface = level.getBlockState(new BlockPos(x, yy + 1, z)).isAir();
                                if (atSurface) {
                                    if (leaks < 5) report.add(String.format("протечка (%d,%d,%d), зеркало %d, дно %d", x, yy, z, top, core.bed()[0]));
                                    leaks++;
                                }
                            }
                        }
                    }
                }
            }
            double dryPct = wet + dry == 0 ? 0.0 : 100.0 * dry / (double) (wet + dry);
            report.add(String.format("протечки: %d (порог 0)", leaks));
            report.add(String.format("сухое русло: %d из %d, %.2f %% (порог 1 %%)", dry, wet + dry, dryPct));
            report.add("вместо воды на зеркале: " + dryBlocks);
            long[] vo2 = measureVo2(level, gen, report);
            long corridor = vo2[0], fit = vo2[1];
            double fitPct = corridor == 0 ? 100.0 : 100.0 * fit / (double) corridor;
            long waterColumns = vo2[4];
            double coveredPct = 100.0 * vo2[5] / Math.max(1, waterColumns);
            long shoreColumns = vo2[2];
            double wallPct = shoreColumns == 0 ? 0.0 : 100.0 * vo2[3] / (double) shoreColumns;
            report.add(String.format("calm_fit: %d из %d, %.2f %% (порог 95 %%)", fit, corridor, fitPct));
            report.add(String.format("накрытая вода: %d из %d колонок, %.2f %% (порог 0,5 %%)",
                    vo2[5], waterColumns, coveredPct));
            report.add(String.format("стены у воды: %d из %d колонок, %.2f %% (порог 2 %%)", vo2[3], shoreColumns, wallPct));
            report.add(String.format("рост текущей воды за %d тиков: %d, %.2f %% источников (порог 5 %%); всего %d, из них чужой (родники, склоны) %d",
                    TICKS, growth, growthPct, growthAll, foreign));
            report.add(String.format("заделки герметизации: %d на %d чанков, %.2f на чанк (порог 1)",
                    seals, chunksWithWater, sealsPerChunk)); report.add(String.format(java.util.Locale.ROOT, "steps: %d, liveSteps=%s, avgTick=%.2f ms, maxTick=%.2f ms", LithoWaterFeature.STEPS.get(), LithoWaterFeature.LIVE_STEPS, avgTickMs, maxTickMs));
            report.add(String.format("время генерации: %.1f мс/чанк (замер Ф5 — в отчёте job'а)", msPerChunk));
            report.add(String.format("вода выше моря: колонок с водой %d, источников %d, текущей %d",
                    wet, sourcesAfter, flowingAfter));
            if (leaks > 0) ok = false;
            if (dryPct > 1.0) ok = false;
            if (growthPct > 5.0) ok = false;
            if (chunksWithWater == 0 || sealsPerChunk > 1.0) ok = false;
            if (corridor == 0 || wet == 0 || fitPct < 95.0) ok = false;
            if (coveredPct > 0.5) ok = false;
            if (wallPct > 2.0) ok = false;
            // Generate this separate regression fixture *after* scoring all
            // original windows. Its water must not dilute a percentage there.
            if (Boolean.parseBoolean(System.getenv("LITHO_CI_WATERFALL_42"))) {
                ok &= checkWaterfall42(level, gen, report);
            }
        } catch (Throwable t) {
            ok = false;
            report.add("FAIL: " + t);
            LithoMod.LOG.error("Litospherity CI water measure", t);
        }
        finish(server, report, ok);
    }

    /** Gameplay seed 42: the two adjacent integer columns in the Rust ABI test. */
    private static boolean checkWaterfall42(ServerLevel level, LithoNative.Generator gen, List<String> report) {
        int x = -44565, z = 6791;
        level.getChunk(x >> 4, z >> 4, ChunkStatus.FULL, true);
        var c = gen.water(new double[]{x, x}, new double[]{z, z + 1});
        int up = c.level()[0], down = c.level()[1];
        boolean face = up == 71 && down == 68 && LithoNative.WaterColumns.flow(c.info()[0]) == 4
                && LithoNative.WaterColumns.flow(c.info()[1]) == 5;
        int falling = 0;
        if (face) {
            for (int y = down; y < up; y++) {
                var state = level.getBlockState(new BlockPos(x, y, z + 1));
                if (containsWater(state) && state.getFluidState().getValue(net.minecraft.world.level.material.FlowingFluid.FALLING)) falling++;
            }
            face = falling == up - down && containsWater(level.getBlockState(new BlockPos(x, up - 1, z)))
                    && containsWater(level.getBlockState(new BlockPos(x, down - 1, z + 1)));
        }
        report.add("водопад (" + x + "," + z + "): " + up + " → " + down + ", падающих блоков " + falling + ", " + (face ? "OK" : "FAIL"));
        return face;
    }

    /** §8.3: inspect each generated column once, after generation and fluid ticks.
     * Include a two-block halo of core predictions, but score only the chunk interior. */
    private static long[] measureVo2(ServerLevel level, LithoNative.Generator gen, List<String> report) {
        final int side = 20;
        long corridor = 0, fit = 0, shore = 0, walls = 0, wet = 0, covered = 0;
        long groundFit = 0;
        var highLand = new java.util.TreeMap<String, Integer>();
        var roofBlocks = new java.util.TreeMap<String, Integer>();
        long[] misses = new long[4]; // dry low/high, wet low/high
        double[] xs = new double[side * side], zs = new double[xs.length];
        for (var chunk : loadedChunks(level)) {
            for (int i = 0; i < xs.length; i++) {
                xs[i] = chunk.getPos().getMinBlockX() + i % side - 2;
                zs[i] = chunk.getPos().getMinBlockZ() + i / side - 2;
            }
            var core = gen.water(xs, zs);
            for (int j = 2; j < 18; j++) {
                for (int i = 2; i < 18; i++) {
                    int k = j * side + i;
                    int lv = core.level()[k];
                    boolean dry = lv == LithoNative.WaterColumns.NONE;
                    int nearby = Integer.MIN_VALUE;
                    boolean exempt = false;
                    if (dry) {
                        for (int dz = -2; dz <= 2; dz++) {
                            for (int dx = -2; dx <= 2; dx++) {
                                if (dx * dx + dz * dz > 4) continue;
                                int n = k + dz * side + dx;
                                if (core.level()[n] < LithoWaterFeature.SEA_LEVEL) continue;
                                nearby = Math.max(nearby, core.level()[n]);
                                exempt |= (core.info()[n] & (1 << 18)) != 0;
                            }
                        }
                    }
                    int expected = core.bed()[k];
                    if (expected == Integer.MIN_VALUE && nearby == Integer.MIN_VALUE) continue;
                    // Do not search just expected +/- 2: that would mistake a block
                    // buried in a tall wall for the top of a well-fitting bank.
                    int top = level.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.OCEAN_FLOOR,
                            (int) xs[k], (int) zs[k]) - 1;
                    if (expected != Integer.MIN_VALUE) {
                        corridor++;
                        // Diagnostic alongside the unchanged raw-height gate. Dry
                        // foliage at VEGETAL_DECORATION is not a density surface.
                        int ground = top;
                        if (dry && top > expected + 2 && level.getBlockState(new BlockPos((int) xs[k], top, (int) zs[k]))
                                .is(net.minecraft.tags.BlockTags.LEAVES)) {
                            while (ground >= level.getMinY() && !level.getBlockState(new BlockPos((int) xs[k], ground, (int) zs[k]))
                                    .is(net.minecraft.tags.BlockTags.BLOCKS_MOTION_IN_HEIGHTMAP_NO_LEAVES)) ground--;
                        }
                        if (Math.abs(ground - expected) <= 2) groundFit++;
                        if (Math.abs(top - expected) <= 2) fit++;
                        else {
                            misses[(dry ? 0 : 2) + (top > expected ? 1 : 0)]++;
                            if (dry && top > expected) highLand.merge(level.getBlockState(
                                    new BlockPos((int) xs[k], top, (int) zs[k])).getBlock().toString(), 1, Integer::sum);
                        }
                    }
                    if (!dry) {
                        wet++;
                        if (top >= lv) {
                            covered++;
                            roofBlocks.merge(level.getBlockState(new BlockPos((int) xs[k], top, (int) zs[k]))
                                    .getBlock().toString(), 1, Integer::sum);
                        }
                    } else if (nearby != Integer.MIN_VALUE && !exempt) {
                        shore++;
                        if (top - nearby >= 4) walls++;
                    }
                }
            }
        }
        report.add(String.format("calm_fit промахи: суша ниже/выше %d/%d, вода ниже/выше %d/%d", misses[0], misses[1], misses[2], misses[3]));
        report.add(String.format("calm_fit без крон (диагностика): %d из %d, %.2f %%", groundFit, corridor, 100.0 * groundFit / Math.max(1, corridor)));
        report.add("верх сухих колонок выше прогноза: " + highLand);
        report.add("верх накрытых колонок: " + roofBlocks);
        return new long[] {corridor, fit, shore, walls, wet, covered};
    }

    // Vegetation runs after RAW_GENERATION. Seagrass and bubble columns
    // still contain source water even though their block is not Blocks.WATER.
    private static boolean containsWater(BlockState state) {
        var fluid = state.getFluidState().getType();
        return fluid == Fluids.WATER || fluid == Fluids.FLOWING_WATER;
    }

    /** Текущая вода выше моря до тиков: ключи {@link BlockPos#asLong()}. */
    private static final Set<Long> FLOWING_BEFORE = new LinkedHashSet<>();

    private static void collectFlowing(ServerLevel level, Set<Long> out) {
        for (net.minecraft.world.level.chunk.LevelChunk chunk : loadedChunks(level)) {
            for (int x = 0; x < 16; x++) {
                for (int z = 0; z < 16; z++) {
                    for (int y = LithoWaterFeature.SEA_LEVEL; y < level.getMaxY(); y++) {
                        BlockPos pos = new BlockPos(chunk.getPos().getMinBlockX() + x, y, chunk.getPos().getMinBlockZ() + z);
                        BlockState state = chunk.getBlockState(pos);
                        if (containsWater(state) && !state.getFluidState().isSource()) out.add(pos.asLong());
                    }
                }
            }
        }
    }

    /** Вода выше моря в загруженных чанках: источники или текущая. */
    private static long countWater(ServerLevel level, boolean sources) {
        long count = 0;
        for (net.minecraft.world.level.chunk.LevelChunk chunk : loadedChunks(level)) {
            for (int x = 0; x < 16; x++) {
                for (int z = 0; z < 16; z++) {
                    for (int y = LithoWaterFeature.SEA_LEVEL + 1; y < level.getMaxY(); y++) {
                        BlockState state = chunk.getBlockState(new BlockPos(
                                chunk.getPos().getMinBlockX() + x, y, chunk.getPos().getMinBlockZ() + z));
                        if (!containsWater(state)) continue;
                        boolean source = state.getFluidState().isSource();
                        if (source == sources) count++;
                    }
                }
            }
        }
        return count;
    }

    /** 26.3: {@code getLoadedChunks()} убран — берём чанки окна по {@code getChunkNow}. */
    private static List<net.minecraft.world.level.chunk.LevelChunk> loadedChunks(ServerLevel level) {
        List<net.minecraft.world.level.chunk.LevelChunk> out = new ArrayList<>();
        for (long key : CHUNKS) {
            net.minecraft.world.level.chunk.LevelChunk chunk =
                    level.getChunkSource().getChunkNow((int) (key >> 32), (int) key);
            if (chunk != null) out.add(chunk);
        }
        return out;
    }

    /** Места из {@code run/water-points.txt}: строки вида {@code x=12 z=-34 level=80}. */
    private static List<int[]> readPoints() {
        List<int[]> points = new ArrayList<>();
        Path file = Path.of("water-points.txt");
        if (!Files.exists(file)) return points;
        try {
            for (String line : Files.readAllLines(file)) {
                String trimmed = line.trim();
                if (!trimmed.startsWith("x=")) continue;
                String[] parts = trimmed.split("\\s+");
                int x = Integer.MIN_VALUE;
                int z = Integer.MIN_VALUE;
                for (String part : parts) {
                    if (part.startsWith("x=")) x = Integer.parseInt(part.substring(2));
                    if (part.startsWith("z=")) z = Integer.parseInt(part.substring(2));
                }
                if (x != Integer.MIN_VALUE && z != Integer.MIN_VALUE) points.add(new int[] {x, z});
                if (points.size() >= 20) break;
            }
        } catch (Exception error) {
            LithoMod.LOG.warn("Litospherity CI: не читаю water-points.txt", error);
        }
        return points;
    }

    private static void finish(MinecraftServer server, List<String> report, boolean ok) {
        report.add(ok ? "RESULT: PASS" : "RESULT: FAIL");
        report.forEach(line -> LithoMod.LOG.info("Litospherity CI water: {}", line));
        try {
            Files.write(Path.of("litospherity-ci.txt"), report);
        } catch (Exception ignored) {
            // отчёт не критичен для выхода сервера
        }
        server.halt(false);
    }
}
