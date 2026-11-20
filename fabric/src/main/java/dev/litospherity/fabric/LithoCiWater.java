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
import net.minecraft.world.level.block.Blocks;
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
        LithoWaterFeature.COVERED.set(0);
        LithoWaterFeature.WALLS.set(0);
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
                        if (generated.add((((long) (cx + dx)) << 32) ^ (cz + dz))) {
                            level.setChunkForced(cx + dx, cz + dz, true);
                        }
                    }
                }
            }
            double ms = (System.nanoTime() - t0) / 1e6;
            report.add(String.format("вода: сгенерировано %d чанков за %.0f мс, %.1f мс/чанк",
                    generated.size(), ms, ms / Math.max(1, generated.size())));
            long sealsAtStart = LithoWaterFeature.SEALS.get();
            long chunksWithWater = LithoWaterFeature.CHUNKS.get();
            long sourcesBefore = countWater(level, true);
            long flowingBefore = countWater(level, false);
            FLOWING_BEFORE.clear();
            collectFlowing(level, FLOWING_BEFORE);
            // Ждём 400 тиков, чтобы обновления жидкости прошли.
            long[] tickStart = {0L}; long[] totalTickNanos = {0L}; long[] maxTickNanos = {0L}; int[] measuredTicks = {0}; boolean[] done = {false}; ServerTickEvents.START_SERVER_TICK.register(s -> { if (!done[0]) tickStart[0] = System.nanoTime(); });
            int[] left = {TICKS};
            ServerTickEvents.END_SERVER_TICK.register(s -> {
                if (done[0]) return; if (tickStart[0] != 0L) { long dt = System.nanoTime() - tickStart[0]; totalTickNanos[0] += dt; if (dt > maxTickNanos[0]) maxTickNanos[0] = dt; measuredTicks[0]++; }
                if (left[0]-- > 0) return; done[0] = true; double avgTickMs = measuredTicks[0] == 0 ? 0.0 : (totalTickNanos[0] / (double) measuredTicks[0]) / 1_000_000.0; double maxTickMs = maxTickNanos[0] / 1_000_000.0;
                measure(server, level, gen, points, report, sealsAtStart, chunksWithWater, sourcesBefore, flowingBefore, ms / Math.max(1, generated.size()), avgTickMs, maxTickMs);
            });
        } catch (Throwable t) {
            report.add("FAIL: " + t);
            LithoMod.LOG.error("Litospherity CI water", t);
            finish(server, report, false);
        }
    }

    private static void measure(MinecraftServer server, ServerLevel level, LithoNative.Generator gen,
                                List<int[]> points, List<String> report, long sealsAtStart, long chunksWithWater,
                                long sourcesBefore, long flowingBefore, double msPerChunk, double avgTickMs, double maxTickMs) {
        boolean ok = true;
        try {
            long flowingAfter = countWater(level, false);
            long sourcesAfter = countWater(level, true);
            long growthAll = flowingAfter - flowingBefore;
            long seals = LithoWaterFeature.SEALS.get() - sealsAtStart;
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
                    String name = st.is(Blocks.WATER) ? (st.getFluidState().isSource() ? "water-src" : "water-flow" + st.getFluidState().getAmount())
                            : net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(st.getBlock()).getPath();
                    line.append(' ').append(name);
                }
                report.add(line.toString());
            }

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
                        if (state.is(Blocks.WATER)) {
                            wet++;
                        } else {
                            dry++;
                        }
                        // Протечка: вода выше поверхности самой колонки. Вода
                        // ядра глубже верха колонки — не протечка: ядро кладёт
                        // источник только туда, где вырез опустил дно под
                        // зеркало, а выше могут остаться воздух или берег.
                        // Раньше окно `top-2…top` считало такие блоки воды
                        // протечкой, хотя это наша же вода (§4 п. 4, Д-127).
                        for (int yy = Math.max(LithoWaterFeature.SEA_LEVEL + 1, y - 2); yy <= y + 1; yy++) {
                            if (!level.getBlockState(new BlockPos(x, yy, z)).is(Blocks.WATER)) continue;
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
                                if (atSurface) leaks++;
                            }
                        }
                    }
                }
            }
            double dryPct = wet + dry == 0 ? 0.0 : 100.0 * dry / (double) (wet + dry);
            report.add(String.format("протечки: %d (порог 0)", leaks));
            report.add(String.format("сухое русло: %d из %d, %.2f %% (порог 1 %%)", dry, wet + dry, dryPct));
            long[] vo2 = measureVo2(level, gen, points);
            long corridor = vo2[0], fit = vo2[1];
            double fitPct = corridor == 0 ? 100.0 : 100.0 * fit / (double) corridor;
            long waterColumns = Math.max(1, LithoWaterFeature.COLUMNS.get());
            double coveredPct = 100.0 * LithoWaterFeature.COVERED.get() / (double) waterColumns;
            long shoreColumns = vo2[2];
            double wallPct = shoreColumns == 0 ? 0.0 : 100.0 * vo2[3] / (double) shoreColumns;
            report.add(String.format("calm_fit: %d из %d, %.2f %% (порог 95 %%)", fit, corridor, fitPct));
            report.add(String.format("накрытая вода: %d из %d колонок, %.2f %% (порог 0,5 %%)",
                    LithoWaterFeature.COVERED.get(), waterColumns, coveredPct));
            report.add(String.format("стены у воды: %d из %d колонок, %.2f %% (порог 2 %%)", vo2[3], shoreColumns, wallPct));
            report.add(String.format("рост текущей воды за %d тиков: %d, %.2f %% источников (порог 5 %%); всего %d, из них чужой (родники, склоны) %d",
                    TICKS, growth, growthPct, growthAll, foreign));
            report.add(String.format("заделки герметизации: %d на %d чанков, %.2f на чанк (порог 2)",
                    seals, chunksWithWater, sealsPerChunk)); report.add(String.format(java.util.Locale.ROOT, "steps: %d, liveSteps=%s, avgTick=%.2f ms, maxTick=%.2f ms", LithoWaterFeature.STEPS.get(), LithoWaterFeature.LIVE_STEPS, avgTickMs, maxTickMs));
            report.add(String.format("время генерации: %.1f мс/чанк (замер Ф5 — в отчёте job'а)", msPerChunk));
            report.add(String.format("вода выше моря: колонок с водой %d, источников %d, текущей %d",
                    wet, sourcesAfter, flowingAfter));
            if (leaks > 0) ok = false;
            if (dryPct > 1.0) ok = false;
            if (growthPct > 5.0) ok = false;
            if (sealsPerChunk > 2.0) ok = false;
            if (fitPct < 95.0) ok = false;
            if (coveredPct > 0.5) ok = false;
            if (wallPct > 2.0) ok = false;
        } catch (Throwable t) {
            ok = false;
            report.add("FAIL: " + t);
            LithoMod.LOG.error("Litospherity CI water measure", t);
        }
        finish(server, report, ok);
    }

    /**
     * ВО2-4 (§8.3): `calm_fit`, накрытая вода и стены по настоящим блокам.
     * Коридор — колонки с водой выше моря и сухие колонки с материалом берега;
     * стены — суша в 2 блоках от воды, где верхний блок выше зеркала на ≥ 4
     * (каньоны/водопады не считаем: у них в `info` не спокойный поток).
     *
     * @return {corridor, fit, shore, walls}
     */
    private static long[] measureVo2(ServerLevel level, LithoNative.Generator gen, List<int[]> points) {
        long corridor = 0;
        long fit = 0;
        long shore = 0;
        long walls = 0;
        int minWorldY = level.getMinY();
        int maxWorldY = level.getMaxY();
        for (int[] p : points) {
            for (int x = (p[0] & ~15) - RADIUS * 16; x < (p[0] & ~15) + (RADIUS + 1) * 16; x++) {
                for (int z = (p[1] & ~15) - RADIUS * 16; z < (p[1] & ~15) + (RADIUS + 1) * 16; z++) {
                    LithoNative.WaterColumns core = gen.water(new double[] {x}, new double[] {z});
                    int lv = core.level()[0];
                    int shoreCode = LithoNative.WaterColumns.shoreMat(core.info()[0]);
                    if (lv != LithoNative.WaterColumns.NONE && lv > LithoWaterFeature.SEA_LEVEL) {
                        corridor++;
                        int expected = core.bed()[0];
                        int top = topSolid(level, x, z, lv - 1, expected - 4, minWorldY, maxWorldY);
                        if (top != Integer.MIN_VALUE && Math.abs(top - expected) <= 2) fit++;
                        continue;
                    }
                    if (shoreCode == 0) continue;
                    corridor++;
                    int expected = core.bed()[0];
                    int top = topSolid(level, x, z, expected + 2, expected - 2, minWorldY, maxWorldY);
                    if (top != Integer.MIN_VALUE && Math.abs(top - expected) <= 2) fit++;
                    // Стены: берег выше воды на ≥ 4. Подмыв (`Underwash`) — это
                    // стенка излучины, класс долины отдельно не экспортируется.
                    if (shoreCode == 9) continue;
                    int waterLevel = Integer.MIN_VALUE;
                    for (int[] d : new int[][] {{1, 0}, {-1, 0}, {0, 1}, {0, -1}, {2, 0}, {-2, 0}, {0, 2}, {0, -2}}) {
                        LithoNative.WaterColumns near = gen.water(new double[] {x + d[0]}, new double[] {z + d[1]});
                        int nl = near.level()[0];
                        if (nl == LithoNative.WaterColumns.NONE) continue;
                        if (LithoNative.WaterColumns.flow(near.info()[0]) != 0) continue;
                        if (nl > LithoWaterFeature.SEA_LEVEL) waterLevel = Math.max(waterLevel, nl);
                    }
                    if (waterLevel == Integer.MIN_VALUE) continue;
                    shore++;
                    if (top != Integer.MIN_VALUE && top - waterLevel >= 4) walls++;
                }
            }
        }
        return new long[] {corridor, fit, shore, walls};
    }

    /** Верхний твёрдый блок (не воздух, не вода) от `fromY` вниз до `toY`. */
    private static int topSolid(ServerLevel level, int x, int z, int fromY, int toY, int minWorldY, int maxWorldY) {
        for (int y = Math.min(fromY, maxWorldY - 1); y >= Math.max(toY, minWorldY); y--) {
            BlockState st = level.getBlockState(new BlockPos(x, y, z));
            if (st.isAir() || st.is(Blocks.WATER)) continue;
            return y;
        }
        return Integer.MIN_VALUE;
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
                        if (state.is(Blocks.WATER) && !state.getFluidState().isSource()) out.add(pos.asLong());
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
                        if (!state.is(Blocks.WATER)) continue;
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
