package dev.litospherity.fabric;

import java.util.concurrent.atomic.AtomicLong;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.ticks.LevelChunkTicks;

/** Ephemeral ownership of static W river blocks. Attached to the protochunk,
 * never written to a save or carried over to the live level. An absent mask
 * (including a reloaded partial chunk) means leave vanilla queues untouched. */
public final class StaticRiverMask {
    public interface Holder { void litospherity$rememberRiver(StaticRiverMask mask); }
    public static final AtomicLong MARKS_REMOVED = new AtomicLong();
    public static final AtomicLong TICKS_REMOVED = new AtomicLong();
    private final int[] bottom = new int[256], top = new int[256];

    StaticRiverMask() { java.util.Arrays.fill(bottom, Integer.MAX_VALUE); }

    void record(int x, int z, int lowestWater, int highestWater) {
        int i = (z & 15) * 16 + (x & 15);
        bottom[i] = lowestWater;
        top[i] = highestWater;
    }

    private boolean owns(ProtoChunk chunk, BlockPos p) {
        if (p.getX() >> 4 != chunk.getPos().x() || p.getZ() >> 4 != chunk.getPos().z()) return false;
        int i = (p.getZ() & 15) * 16 + (p.getX() & 15);
        return p.getY() >= bottom[i] && p.getY() <= top[i] && chunk.getBlockState(p).is(Blocks.WATER);
    }

    /** Vanilla converts two independent queues at FULL: marks for immediate
     * fluid ticks, and SavedTicks scheduled during later decoration. Do not
     * touch block ticks, waterlogged vegetation, foreign fluids or play-time
     * neighbour updates. Retain the original queue if nothing was removed. */
    public LevelChunkTicks<Fluid> finish(ProtoChunk chunk, LevelChunkTicks<Fluid> original) {
        var sections = chunk.getPostProcessing();
        int removed = 0;
        for (int i = 0; i < sections.length; i++) {
            if (sections[i] == null) continue;
            var offsets = sections[i].iterator();
            while (offsets.hasNext()) {
                var p = ProtoChunk.unpackOffsetCoordinates(offsets.nextShort(), chunk.getSectionYFromSectionIndex(i), chunk.getPos());
                if (owns(chunk, p)) { offsets.remove(); removed++; }
            }
        }
        MARKS_REMOVED.addAndGet(removed);
        if (original.count() == 0) return original;
        // At this hook the queue contains only still-packed protochunk ticks.
        var saved = original.pack(0L);
        var kept = saved.stream().filter(t -> (t.type() != Fluids.WATER && t.type() != Fluids.FLOWING_WATER)
                || !owns(chunk, t.pos())).toList();
        if (saved.size() == kept.size()) return original;
        TICKS_REMOVED.addAndGet(saved.size() - kept.size());
        return new LevelChunkTicks<>(kept);
    }
}
