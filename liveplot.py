"""

Demonstrates the use of multiple Progress instances in a single Live display.    

"""

import sys
import json
from time import sleep

from rich.live import Live
from rich.panel import Panel
from rich.progress import Progress, SpinnerColumn, BarColumn, TextColumn
from rich.table import Table

job_progress=None

progress_table = Table.grid()

with Live(progress_table, refresh_per_second=10):
    for line in sys.stdin:
        line=line[:-1]
        if job_progress is None:
            job_progress = Progress("{task.description}",BarColumn(),TextColumn("[progress.percentage]{task.percentage:>3.0f}%"),)
            ddpp=[dp.split(':') for dp in json.loads(line)]
            oneDev=(len(set([dp[0] for dp in ddpp]))==1)
            for dp in ddpp: job_progress.add_task(dp[1] if oneDev else ':'.join(dp),total=1.0)
            progress_table.add_row(Panel.fit(job_progress, title=(f'[b]{ddpp[0][0]}' if oneDev else 'VU meter'), border_style="red", padding=(1, 2)))
            continue
        vv=[float(v) for v in line.split()]
        for job,val in zip(job_progress.tasks,vv):
            job_progress.update(job.id,completed=val)
