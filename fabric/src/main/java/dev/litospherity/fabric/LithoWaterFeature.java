package dev.litospherity.fabric;

import com.mojang.serialization.MapCodec;
import java.util.concurrent.atomic.AtomicLong;
import net.minecraft.core.BlockPos;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.material.Fluids;

/**
 * Ф6-В3/В4 (PLAN-WATER-FABRIC): вода выше уровня моря на Fabric.
 *
 * <p>Заливка по колонкам и герметизация одним проходом на шаге {@code RAW_GENERATION}
 * (запасной путь раздела 3 плана): синхронная точка заливки {@code NoiseBasedChunkGenerator}
 * в 26.3 без компилятора не проверяется, а фича — обычный API, который переживает C2ME.
 * Плата: дно под водой правится здесь же, а не шагом SURFACE.
 *
 * <ul>
 *   <li>уровень и дно колонки берутся у ядра ({@code litho_water_batch}), один вызов FFM на чанк
 *       (18×18 колонок: чанк плюс блок по краю для герметизации);</li>
 *   <li>вода ставится от {@code level − 1} вниз до дна, пока блок — воздух;</li>
 *   <li>если сосед по стороне — воздух, а у ядра воды на этой высоте нет, туда ставится блок
 *       берега (заделка): это метрика, а не норма (шаг В5);</li>
 *   <li>верхний блок под водой меняется на землю, как у подводного дна.</li>
 * </ul>
 *
 * <p>Работает только в мире с пресетом Litospherity ({@code v1-dev-r2}); в остальных мирах
 * возвращает {@code false}, и вода выше моря не появляется.
 */
public final class LithoWaterFeature implements Feature {
    /** Уровень моря: выше него — предмет R2. */
    public static final int SEA_LEVEL = 63;
    /** Колонка чанка плюс один блок по краю: герметизации нужны соседи. */
    private static final int SIDE = 18;

    /** Заделок герметизации (метрика В5: ≤ 2 на чанк с водой выше моря). */
    public static final AtomicLong SEALS = new AtomicLong();
    /** Чанков, где заливалась вода выше моря. */
    public static final AtomicLong CHUNKS = new AtomicLong();
    /** Колонок, где вода выше моря поставлена. */
    public static final AtomicLong COLUMNS = new AtomicLong();
    private static final byte KIND_RIVER = 1;
    public static volatile boolean LIVE_STEPS = Boolean.getBoolean("litospherity.water.liveSteps");
    public static final AtomicLong STEPS = new AtomicLong();

    /** 26.3: {@code Feature} — интерфейс без параметра конфигурации, а реестр
     *  {@code FEATURE_TYPE} держит кодеки. Типу воды конфигурация не нужна. */
    public static final MapCodec<LithoWaterFeature> CODEC = MapCodec.unit(new LithoWaterFeature());

    @Override
    public MapCodec<LithoWaterFeature> codec() {
        return CODEC;
    }

    @Override
    public boolean place(WorldGenLevel level, ChunkGenerator chunkGenerator, RandomSource random, BlockPos origin) {
        LithoFields.Binding binding = LithoFields.world();
        if (binding == null || binding.version() != LithoFields.VERSION_R2) return false;
        int x0 = origin.getX() & ~15;
        int z0 = origin.getZ() & ~15;
        // Один вызов ядра на чанк: 18×18 колонок, начиная с блока левее/выше чанка.
        double[] xs = new double[SIDE * SIDE];
        double[] zs = new double[SIDE * SIDE];
        for (int j = 0; j < SIDE; j++) {
            for (int i = 0; i < SIDE; i++) {
                xs[j * SIDE + i] = x0 - 1 + i;
                zs[j * SIDE + i] = z0 - 1 + j;
            }
        }
        // Генератор общий и потокобезопасный (Ф1): не закрываем его здесь.
        LithoNative.Generator core = LithoFields.generator(binding);
        // Дешёвая проба: 4x4 колонки через 5 блоков. Чанк без воды выше моря
        // не платит за полный пакет 18x18 (цена колонки ядра — десятки мкс).
        double[] probeX = new double[16];
        double[] probeZ = new double[16];
        for (int j = 0; j < 4; j++) {
            for (int i = 0; i < 4; i++) {
                probeX[j * 4 + i] = x0 + i * 5;
                probeZ[j * 4 + i] = z0 + j * 5;
            }
        }
        LithoNative.WaterColumns probe = core.water(probeX, probeZ);
        boolean any = false;
        for (int probeLevel : probe.level()) {
            if (probeLevel != LithoNative.WaterColumns.NONE && probeLevel > SEA_LEVEL) any = true;
        }
        if (!any) return false;
        LithoNative.WaterColumns water = core.water(xs, zs);
        int minY = level.getMinY();
        int maxY = level.getMaxY();
        int placed = 0;
        long seals = 0;
        long steps = 0;
        BlockState waterState = Blocks.WATER.defaultBlockState();
        // Заливка и правка дна.
        for (int j = 1; j <= 16; j++) {
            for (int i = 1; i <= 16; i++) {
                int k = j * SIDE + i;
                int surface = water.level()[k];
                if (surface == LithoNative.WaterColumns.NONE || surface <= SEA_LEVEL) continue;
                int x = (int) xs[k];
                int z = (int) zs[k];
                int from = Math.min(surface - 1, maxY);
                int to = Math.max(water.bed()[k] - 2, minY);
                // §4 пп. 3–4: колонка с водой выше моря не бывает сухой. Зеркало
                // `level` — это срез: всё, что стоит между ним и дном русла,
                // вынимается, а вода ставится от `level − 1` вниз. Раньше цикл
                // останавливался на первом не-воздухе, поэтому трава, дресва и
                // земля над зеркалом давали «сухое русло» — 20,6 % в научном
                // режиме против порога 1 % (job `water`).
                //
                // Глубже `bed − 2` не режем никогда: дно задаёт ядро, а пещеры
                // под руслом закрывает затычка ниже.
                for (int y = from; y >= to; y--) {
                    BlockPos pos = new BlockPos(x, y, z);
                    if (level.getBlockState(pos).isAir()) continue;
                    level.setBlock(pos, Blocks.AIR.defaultBlockState(), 2);
                }
                int lowest = from;
                for (int y = from; y >= to; y--) {
                    BlockPos pos = new BlockPos(x, y, z);
                    if (!level.getBlockState(pos).isAir()) break;
                    level.setBlock(pos, waterState, 2);
                    lowest = y;
                }
                if (lowest < from) {
                    placed++;
                    COLUMNS.incrementAndGet();
                    // Дно под водой: трава и подобное становятся землёй.
                    if (lowest - 1 >= minY) {
                        BlockPos bed = new BlockPos(x, lowest - 1, z);
                        if (isSurface(level.getBlockState(bed))) {
                            level.setBlock(bed, Blocks.DIRT.defaultBlockState(), 2);
                        }
                    }
                    // Затычка: под водой остался воздух (пещера под руслом).
                    BlockPos under = new BlockPos(x, lowest - 1, z);
                    if (lowest - 1 >= minY && level.getBlockState(under).isAir()) {
                        level.setBlock(under, Blocks.STONE.defaultBlockState(), 2);
                        seals++;
                    }
                }
            }
        }
        // Герметизация: вода не должна уходить в воздух за берегом.
        for (int j = 1; j <= 16; j++) {
            for (int i = 1; i <= 16; i++) {
                int k = j * SIDE + i;
                int surface = water.level()[k];
                if (surface == LithoNative.WaterColumns.NONE || surface <= SEA_LEVEL) continue;
                for (int[] step : new int[][] {{1, 0}, {-1, 0}, {0, 1}, {0, -1}}) {
                    int nk = (j + step[1]) * SIDE + (i + step[0]);
                    int neighbour = water.level()[nk];
                    boolean riverStep = water.kind()[k] == KIND_RIVER && water.kind()[nk] == KIND_RIVER && neighbour != LithoNative.WaterColumns.NONE && neighbour > SEA_LEVEL && neighbour < surface;
                    int top = Math.min(surface - 1, maxY);
                    int bottom = Math.max(water.bed()[k] - 2, minY);
                    for (int y = top; y >= bottom; y--) {
                        if (neighbour != LithoNative.WaterColumns.NONE && neighbour > y) continue;
                        BlockPos pos = new BlockPos((int) xs[nk], y, (int) zs[nk]);
                        if (!level.getBlockState(pos).isAir()) continue;
                        if (riverStep) {
                            BlockState flowing = Fluids.FLOWING_WATER.getFlowing(7, y < top).createLegacyBlock();
                            level.setBlock(pos, flowing, 2);
                            // ChunkAccess не умеет постобработку; она есть у ProtoChunk, а фича
                            // RAW_GENERATION идёт именно на нём.
                            if (LIVE_STEPS && level.getChunk(pos) instanceof ProtoChunk protoChunk) {
                                protoChunk.markPosForPostprocessing(pos);
                            }
                            steps++;
                        } else {
                            level.setBlock(pos, Blocks.DIRT.defaultBlockState(), 2);
                            seals++;
                        }
                    }
                }
            }
        }
        if (placed > 0) CHUNKS.incrementAndGet();
        SEALS.addAndGet(seals);
        STEPS.addAndGet(steps);
        return placed > 0;
    }

    private static boolean isSurface(BlockState state) {
        return state.is(Blocks.GRASS_BLOCK)
                || state.is(Blocks.PODZOL)
                || state.is(Blocks.MYCELIUM)
                || state.is(Blocks.COARSE_DIRT)
                || state.is(Blocks.ROOTED_DIRT)
                || state.is(Blocks.MOSS_BLOCK)
                || state.is(Blocks.SNOW_BLOCK)
                || state.is(Blocks.DIRT_PATH)
                || state.is(Blocks.FARMLAND)
                || state.is(Blocks.MUD)
                || state.is(Blocks.SAND);
    }
}
